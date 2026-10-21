use super::{fail, Result};
use serde_json::{json, Map, Value};
use std::{collections::BTreeSet, fs, path::Path};

fn read(path: &Path, what: &str) -> Result<Value> {
    if !path.exists() {
        return Err(fail(
            3,
            format!("no {what} at {} — run the suite first", path.display()),
        ));
    }
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}
pub(super) fn platform() -> String {
    std::env::var("ONECAD_BENCH_PLATFORM")
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| {
            let os = if std::env::consts::OS == "macos" {
                "darwin"
            } else {
                std::env::consts::OS
            };
            let arch = match std::env::consts::ARCH {
                "aarch64" => "arm64",
                "x86_64" => "x64",
                "x86" => "ia32",
                other => other,
            };
            format!("{os}-{arch}")
        })
}
pub(super) fn digests(directory: &Path, suite: &str, platform: &str) -> Result<Map<String, Value>> {
    let path = directory.join("results.jsonl");
    if !path.exists() {
        return Err(fail(
            3,
            format!("no results at {} — run the suite first", path.display()),
        ));
    }
    let mut rows = Map::new();
    for line in fs::read_to_string(path)?
        .lines()
        .filter(|line| !line.trim().is_empty())
    {
        let row: Value = serde_json::from_str(line)?;
        let variant = row["campaignVariant"]["name"].as_str().unwrap_or("base");
        let field = |key: &str| row[key].as_str().unwrap_or("undefined");
        rows.insert(
            format!(
                "{suite}|{}|{}|{variant}|{platform}",
                field("caseId"),
                field("backend")
            ),
            json!({"inputDigest":row["inputDigest"],"normalizedDigest":row["normalizedDigest"]}),
        );
    }
    Ok(rows)
}
fn scoped(key: &str, suite: &str, platform: &str) -> bool {
    key.starts_with(&format!("{suite}|")) && key.ends_with(&format!("|{platform}"))
}
pub(super) fn semantics(directory: &Path) -> Result<Value> {
    let source = read(&directory.join("summary.json"), "summary")?;
    let mut fields = Map::new();
    for name in [
        "schemaVersion",
        "records",
        "gatingFailures",
        "counts",
        "differential",
        "replay",
        "metamorph",
        "criticalTransitions",
    ] {
        fields.insert(name.into(), source[name].clone());
    }
    Ok(Value::Object(fields))
}
pub(super) fn diff(
    want: Option<&Value>,
    got: Option<&Value>,
    path: &str,
    problems: &mut Vec<String>,
) {
    if let (Some(Value::Object(want)), Some(Value::Object(got))) = (want, got) {
        let keys: BTreeSet<_> = want.keys().chain(got.keys()).collect();
        for key in keys {
            let path = if path.is_empty() {
                key.clone()
            } else {
                format!("{path}.{key}")
            };
            diff(want.get(key), got.get(key), &path, problems);
        }
    } else if want != got {
        let value = |value: Option<&Value>| value.map_or("undefined".into(), Value::to_string);
        problems.push(format!(
            "{path}: expected {}, got {}",
            value(want),
            value(got)
        ));
    }
}
pub(super) fn digest_diff(
    expected: &Map<String, Value>,
    observed: &Map<String, Value>,
) -> Vec<String> {
    let mut problems = Vec::new();
    for (key, want) in expected {
        let Some(got) = observed.get(key) else {
            problems.push(format!("MISSING  {key}"));
            continue;
        };
        for field in ["inputDigest", "normalizedDigest"] {
            if got[field] != want[field] {
                let display = |value: &Value| {
                    value
                        .as_str()
                        .map_or_else(|| value.to_string(), str::to_string)
                };
                problems.push(format!(
                    "CHANGED  {key} {field}\n           was {}\n           now {}",
                    display(&want[field]),
                    display(&got[field])
                ));
            }
        }
    }
    for key in observed.keys() {
        if !expected.contains_key(key) {
            problems.push(format!("UNEXPECTED {key}"));
        }
    }
    problems
}
fn write(path: &Path, object: Map<String, Value>, label: &str) -> Result<()> {
    fs::write(
        path,
        format!("{}\n", serde_json::to_string_pretty(&object)?),
    )?;
    println!("{label}");
    Ok(())
}
pub(super) fn run(args: &[String]) -> Result<()> {
    let modes = ["record", "compare", "semantic-record", "semantic-compare"];
    if args.len() < 4
        || !modes.contains(&args[0].as_str())
        || args[1..4].iter().any(String::is_empty)
    {
        return Err(fail(2,"usage: kernelbench-manifest <record|compare|semantic-record|semantic-compare> <out-dir> <suite> <manifest.json>"));
    }
    let (mode, directory, suite, path) =
        (&args[0], Path::new(&args[1]), &args[2], Path::new(&args[3]));
    let platform = platform();
    if mode.ends_with("record") {
        let mut baseline = if path.exists() {
            read(path, "baseline")?
                .as_object()
                .ok_or("baseline must be an object")?
                .clone()
        } else {
            Map::new()
        };
        if mode == "record" {
            let observed = digests(directory, suite, &platform)?;
            baseline.retain(|key, _| !scoped(key, suite, &platform));
            let count = observed.len();
            baseline.extend(observed);
            return write(
                path,
                baseline,
                &format!(
                    "recorded {count} rows for {suite} on {platform} → {}",
                    path.display()
                ),
            );
        }
        baseline.insert(suite.clone(), semantics(directory)?);
        return write(
            path,
            baseline,
            &format!("recorded semantics for {suite} → {}", path.display()),
        );
    }
    compare(mode, directory, suite, path, &platform)
}
fn compare(mode: &str, directory: &Path, suite: &str, path: &Path, platform: &str) -> Result<()> {
    if mode == "compare" && !path.exists() {
        return Err(fail(
            3,
            format!("no manifest at {} — record one first", path.display()),
        ));
    }
    let baseline = read(
        path,
        if mode == "compare" {
            "manifest"
        } else {
            "semantics baseline"
        },
    )?;
    let problems = if mode == "semantic-compare" {
        let expected = baseline
            .get(suite)
            .filter(|value| !value.is_null())
            .ok_or_else(|| {
                fail(
                    3,
                    format!("no semantics baseline for {suite} — semantic-record one first"),
                )
            })?;
        let mut problems = Vec::new();
        diff(
            Some(expected),
            Some(&semantics(directory)?),
            "",
            &mut problems,
        );
        problems
    } else {
        let observed = digests(directory, suite, platform)?;
        let expected: Map<_, _> = baseline
            .as_object()
            .ok_or("baseline must be an object")?
            .iter()
            .filter(|(key, _)| scoped(key, suite, platform))
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
        if expected.is_empty() {
            return Err(fail(
                3,
                format!(
                    "no digest rows for {suite} on {platform} in {} — record this platform first",
                    path.display()
                ),
            ));
        }
        let problems = digest_diff(&expected, &observed);
        if problems.is_empty() {
            println!(
                "kernelbench manifest OK for {suite} on {platform}: {} rows unchanged",
                expected.len()
            );
            return Ok(());
        }
        problems
    };
    if !problems.is_empty() {
        let label = if mode == "compare" {
            "manifest MISMATCH"
        } else {
            "SEMANTIC mismatch"
        };
        return Err(fail(
            1,
            format!(
                "kernelbench {label} for {suite} on {platform} ({}):\n  {}",
                problems.len(),
                problems.join("\n  ")
            ),
        ));
    }
    println!("kernelbench semantics OK for {suite} on {platform}");
    Ok(())
}
