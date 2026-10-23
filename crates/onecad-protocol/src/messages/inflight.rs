//! Bounded active-job telemetry for the two execution lanes.
use serde::{de, Deserialize, Deserializer, Serialize};

use super::InflightResult;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum JobKind {
    Kernel,
    Solver,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackedJob {
    pub kind: JobKind,
    #[serde(flatten)]
    pub job: InflightResult,
}

pub fn deserialize_jobs<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<TrackedJob>, D::Error> {
    struct Jobs;
    impl<'de> de::Visitor<'de> for Jobs {
        type Value = Vec<TrackedJob>;
        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("at most two distinct active kernel/solver jobs")
        }
        fn visit_seq<A: de::SeqAccess<'de>>(
            self,
            mut sequence: A,
        ) -> Result<Self::Value, A::Error> {
            let mut jobs: Vec<TrackedJob> = Vec::with_capacity(2);
            while let Some(record) = sequence.next_element::<TrackedJob>()? {
                if jobs.len() == 2
                    || record.job.id == 0
                    || record.job.verb.is_empty()
                    || record.job.verb.len() > 128
                    || record.job.since_progress_ms > record.job.age_ms
                    || jobs
                        .iter()
                        .any(|old| old.kind == record.kind || old.job.id == record.job.id)
                {
                    return Err(de::Error::custom(
                        "invalid active-job identity, clock or capacity",
                    ));
                }
                jobs.push(record);
            }
            Ok(jobs)
        }
    }
    deserializer.deserialize_seq(Jobs)
}

pub fn decode_jobs(value: serde_json::Value) -> Result<Vec<TrackedJob>, serde_json::Error> {
    #[derive(Deserialize)]
    struct Jobs(#[serde(deserialize_with = "deserialize_jobs")] Vec<TrackedJob>);
    serde_json::from_value::<Jobs>(value).map(|jobs| jobs.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn job(kind: &str, id: u64) -> serde_json::Value {
        json!({"kind":kind,"verb":"SketchUpsert","id":id,"ageMs":100,"sinceProgressMs":100})
    }

    #[test]
    fn independent_jobs_decode_and_invalid_snapshots_fail_closed() {
        assert_eq!(
            decode_jobs(json!([job("kernel", 1), job("solver", 2)]))
                .unwrap()
                .len(),
            2
        );
        for invalid in [
            json!(null),
            json!([job("solver", 0)]),
            json!([job("kernel", 1), job("solver", 1)]),
            json!([job("kernel", 1), job("kernel", 2)]),
            json!([job("kernel", 1), job("solver", 2), job("solver", 3)]),
            json!([{"kind":"solver","verb":"SketchRegions","id":2,"ageMs":1,"sinceProgressMs":2}]),
            json!([{"kind":"future","verb":"SketchRegions","id":2,"ageMs":1,"sinceProgressMs":1}]),
        ] {
            assert!(decode_jobs(invalid.clone()).is_err(), "{invalid}");
        }
    }
}
