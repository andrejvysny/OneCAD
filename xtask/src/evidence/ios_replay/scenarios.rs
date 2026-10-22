use super::*;

pub(super) fn chronology(scenario: &Scenario) -> Result<[f64; 2]> {
    let before = state(&scenario.observations[2])?;
    let after = state(&scenario.observations[4])?;
    check(
        before.live_contacts == 0 && before.pose.is_null() && before.prediction.is_none(),
        "chronology did not start cancelled",
    )?;
    let samples = chronology_samples(&scenario.observations[3])?;
    check(
        after.live_contacts == 1
            && after.pencil_position == Some(samples[0].position())
            && after.prediction == Some(samples[4].position()),
        "real or predicted chronology endpoint differs",
    )?;
    pose(&after, 0.25, 1)?;
    delta(&before, &after, "duplicateCoalesced", 1)?;
    for key in [
        "invalid",
        "stale",
        "unknownContact",
        "oldGeneration",
        "refusedContacts",
    ] {
        delta(&before, &after, key, 0)?;
    }
    Ok(samples[0].position())
}

fn chronology_samples(observation: &Observation) -> Result<Vec<Sample>> {
    let samples = samples(observation)?;
    check(samples.len() == 5, "chronology needs five unsorted samples")?;
    let phases = [
        ("moved", "actual"),
        ("moved", "coalesced"),
        ("began", "actual"),
        ("moved", "coalesced"),
        ("moved", "predicted"),
    ];
    for (sample, (phase, provenance)) in samples.iter().zip(phases) {
        check(
            sample.phase == phase && sample.provenance == provenance,
            "chronology sample order differs",
        )?;
    }
    let mut duplicate = samples[3].clone();
    duplicate.provenance = "actual".into();
    check(
        duplicate == samples[0]
            && samples[2].t < samples[1].t
            && samples[1].t < samples[0].t
            && samples[0].t < samples[4].t
            && samples[0].position() != samples[1].position()
            && samples[0].position() != samples[2].position()
            && samples[0].position() != samples[4].position(),
        "coalesced duplicate or timestamp chronology missing",
    )?;
    check(
        samples[0].estimation_index == Some(102)
            && samples[1].estimation_index == Some(101)
            && samples[2].estimation_index == Some(100)
            && samples[0].force == Some(0.25)
            && samples[0].max_force == Some(1.0)
            && samples[0].estimated == ["force"]
            && samples[0].expects_update == ["force"],
        "initial estimated force sample differs",
    )?;
    Ok(samples)
}

pub(super) fn predictions(scenario: &Scenario, actual: &[f64; 2]) -> Result<()> {
    let before = state(&scenario.observations[0])?;
    let first = state(&scenario.observations[2])?;
    let after = state(&scenario.observations[4])?;
    let first_samples = samples(&scenario.observations[1])?;
    let replacement = samples(&scenario.observations[3])?;
    check(
        first_samples.len() == 2 && replacement.len() == 1,
        "prediction batches differ",
    )?;
    for sample in first_samples.iter().chain(&replacement) {
        check(
            sample.phase == "moved" && sample.provenance == "predicted",
            "prediction provenance differs",
        )?;
    }
    check(
        first.prediction == Some(first_samples[1].position())
            && after.prediction == Some(replacement[0].position())
            && before.prediction != first.prediction
            && first.prediction != after.prediction,
        "prediction was not replaced",
    )?;
    for observed in [&first, &after] {
        check(
            observed.live_contacts == 1
                && observed.pencil_position.as_ref() == Some(actual)
                && observed.pose == before.pose
                && observed.stats == before.stats,
            "prediction mutated real pose, contact or counters",
        )?;
    }
    Ok(())
}

pub(super) fn estimation(scenario: &Scenario, actual: &[f64; 2]) -> Result<()> {
    let before = state(&scenario.observations[0])?;
    let after = state(&scenario.observations[2])?;
    let samples = samples(&scenario.observations[1])?;
    check(samples.len() == 1, "estimation needs one correction")?;
    let sample = &samples[0];
    check(
        sample.phase == "moved"
            && sample.provenance == "estimation_update"
            && sample.estimation_index == Some(102)
            && sample.force == Some(0.75)
            && sample.max_force == Some(1.0)
            && sample.estimated.is_empty()
            && sample.expects_update.is_empty()
            && &sample.position() != actual,
        "force correction sample differs",
    )?;
    pose(&after, 0.75, 0)?;
    check(
        after.live_contacts == 1
            && after.pencil_position.as_ref() == Some(actual)
            && after.prediction == before.prediction,
        "force correction moved or cleared real input",
    )?;
    delta(&before, &after, "estimationMatched", 1)?;
    delta(&before, &after, "estimationUnmatched", 0)
}

pub(super) fn cancellation(scenario: &Scenario, actual: &[f64; 2]) -> Result<()> {
    let before = state(&scenario.observations[0])?;
    let cancelled = state(&scenario.observations[2])?;
    let late = state(&scenario.observations[4])?;
    check(
        before.live_contacts == 1 && before.prediction.is_some(),
        "no live predicted contact to cancel",
    )?;
    let samples = samples(&scenario.observations[3])?;
    check(
        samples.len() == 1 && samples[0].phase == "ended" && samples[0].provenance == "actual",
        "missing late actual ended sample",
    )?;
    check(
        cancelled.live_contacts == 0
            && late.live_contacts == 0
            && cancelled.prediction.is_none()
            && late.prediction.is_none()
            && late.pencil_position.as_ref() == Some(actual),
        "cancellation or late-end rejection failed",
    )?;
    pose(&late, 0.75, 0)?;
    delta(&cancelled, &late, "oldGeneration", 1)
}
