//! One test group per INPUT.md normalizer rule (names start with `rule<N>_`).
use super::test_support::*;
use super::*;
use Phase::*;

fn contacts(events: &[Normalized]) -> Vec<(ContactId, Phase, f64)> {
    events
        .iter()
        .filter_map(|event| match event {
            Normalized::Contact(s) => {
                Some((s.contact, s.phase, s.position.map_or(f64::NAN, |p| p.x)))
            }
            _ => None,
        })
        .collect()
}

fn submit(normalizer: &mut Normalizer, samples: &[Sample]) -> Vec<Normalized> {
    normalizer.submit(samples, 0).expect("bounded batch");
    normalizer.drain()
}

#[test]
fn rule1_samples_are_delivered_in_timestamp_order_stable_for_equal_times() {
    let mut normalizer = Normalizer::default();
    let events = submit(
        &mut normalizer,
        &[
            pencil(1, Began, 0.0, 0.0, 0.0),
            finger(2, Began, 0.0, 200.0, 0.0),
            pencil(1, Moved, 0.2, 2.0, 0.0),
            with(Provenance::Coalesced, pencil(1, Moved, 0.1, 1.0, 0.0)),
            finger(2, Moved, 0.1, 201.0, 0.0),
        ],
    );
    assert_eq!(
        contacts(&events),
        [
            (1, Began, 0.0),
            (2, Began, 200.0),
            (1, Moved, 1.0),
            (2, Moved, 201.0),
            (1, Moved, 2.0)
        ]
    );
}

#[test]
fn rule2_the_coalesced_copy_of_an_actual_sample_is_delivered_once() {
    let mut normalizer = Normalizer::default();
    submit(&mut normalizer, &[pencil(1, Began, 0.0, 0.0, 0.0)]);
    let events = submit(
        &mut normalizer,
        &[
            with(Provenance::Coalesced, pencil(1, Moved, 0.1, 1.0, 0.0)),
            with(Provenance::Coalesced, pencil(1, Moved, 0.2, 2.0, 0.0)),
            pencil(1, Moved, 0.2, 2.0, 0.0),
        ],
    );
    assert_eq!(contacts(&events), [(1, Moved, 1.0), (1, Moved, 2.0)]);
    assert_eq!(normalizer.stats().duplicate_coalesced, 1);
}

#[test]
fn rule3_predictions_are_replaced_per_batch_and_never_contact_samples() {
    let mut normalizer = Normalizer::default();
    let predicted = |t, x| with(Provenance::Predicted, pencil(1, Moved, t, x, 0.0));
    let first = submit(
        &mut normalizer,
        &[
            pencil(1, Began, 0.0, 0.0, 0.0),
            predicted(0.1, 5.0),
            predicted(0.2, 6.0),
        ],
    );
    assert_eq!(contacts(&first), [(1, Began, 0.0)]);
    assert_eq!(
        first.last(),
        Some(&Normalized::Prediction {
            contact: 1,
            points: vec![p(5.0, 0.0), p(6.0, 0.0)]
        })
    );
    let second = submit(
        &mut normalizer,
        &[pencil(1, Moved, 0.1, 1.0, 0.0), predicted(0.3, 9.0)],
    );
    assert_eq!(
        second.last(),
        Some(&Normalized::Prediction {
            contact: 1,
            points: vec![p(9.0, 0.0)]
        })
    );
    // A batch without predictions clears the list.
    let third = submit(&mut normalizer, &[pencil(1, Moved, 0.2, 2.0, 0.0)]);
    assert_eq!(
        third.last(),
        Some(&Normalized::Prediction {
            contact: 1,
            points: vec![]
        })
    );
}

#[test]
fn rule3_an_ended_or_cancelled_contact_clears_its_predictions() {
    for end in [Ended, Cancelled] {
        let mut normalizer = Normalizer::default();
        let predicted = with(Provenance::Predicted, pencil(1, Moved, 0.1, 5.0, 0.0));
        submit(
            &mut normalizer,
            &[pencil(1, Began, 0.0, 0.0, 0.0), predicted],
        );
        let events = submit(&mut normalizer, &[pencil(1, end, 0.2, 1.0, 0.0), predicted]);
        assert!(events.contains(&Normalized::Prediction {
            contact: 1,
            points: vec![]
        }));
        assert!(!events.iter().any(|event| matches!(
            event,
            Normalized::Prediction { points, .. } if !points.is_empty()
        )));
    }
}

fn indexed(sample: Sample, index: u64, force: Option<f64>) -> Sample {
    Sample {
        estimation_index: Some(index),
        pose: Pose {
            force,
            estimated: mask::FORCE,
            expects_update: mask::FORCE,
            ..Pose::default()
        },
        ..sample
    }
}

#[test]
fn rule4_estimation_updates_refine_the_retained_pose_without_events() {
    let mut normalizer = Normalizer::default();
    submit(
        &mut normalizer,
        &[indexed(pencil(1, Began, 0.0, 0.0, 0.0), 7, Some(0.1))],
    );
    submit(&mut normalizer, &[pencil(1, Ended, 0.1, 0.0, 0.0)]);
    // UIKit refines force after the touch ended; the position must not move.
    let mut update = indexed(
        with(
            Provenance::EstimationUpdate,
            pencil(1, Ended, 0.3, 50.0, 50.0),
        ),
        7,
        Some(0.8),
    );
    update.pose.estimated = 0;
    update.pose.expects_update = 0;
    let events = submit(&mut normalizer, &[update]);
    assert!(events.is_empty(), "{events:?}");
    let pose = normalizer.pose(1, 7).expect("retained");
    assert_eq!(
        (pose.force, pose.estimated, pose.expects_update),
        (Some(0.8), 0, 0)
    );
    assert_eq!(normalizer.stats().estimation_matched, 1);
}

#[test]
fn rule4_unmatched_updates_are_counted_and_dropped() {
    let mut normalizer = Normalizer::default();
    submit(
        &mut normalizer,
        &[indexed(pencil(1, Began, 0.0, 0.0, 0.0), 1, Some(0.1))],
    );
    let update = indexed(
        with(
            Provenance::EstimationUpdate,
            pencil(1, Moved, 0.1, 0.0, 0.0),
        ),
        99,
        Some(1.0),
    );
    assert!(submit(&mut normalizer, &[update]).is_empty());
    assert_eq!(normalizer.stats().estimation_unmatched, 1);
}

#[test]
fn rule4_the_estimation_ring_keeps_the_newest_256_per_contact() {
    let mut normalizer = Normalizer::default();
    submit(
        &mut normalizer,
        &[indexed(pencil(1, Began, 0.0, 0.0, 0.0), 0, None)],
    );
    for i in 1..=300u64 {
        let t = i as f64;
        submit(
            &mut normalizer,
            &[indexed(pencil(1, Moved, t, t, 0.0), i, None)],
        );
    }
    assert!(normalizer.pose(1, 300 - 255).is_some());
    assert!(normalizer.pose(1, 300 - 256).is_none());
}

#[test]
fn rule5_stale_non_terminal_samples_drop_but_terminal_ones_apply() {
    let mut normalizer = Normalizer::default();
    submit(
        &mut normalizer,
        &[
            pencil(1, Began, 1.0, 0.0, 0.0),
            pencil(1, Moved, 2.0, 2.0, 0.0),
        ],
    );
    let events = submit(
        &mut normalizer,
        &[
            pencil(1, Moved, 1.5, 1.0, 0.0),
            pencil(1, Ended, 1.6, 3.0, 0.0),
        ],
    );
    assert_eq!(contacts(&events), [(1, Ended, 3.0)]);
    assert_eq!(normalizer.stats().stale, 1);
}

#[test]
fn rule6_samples_of_an_unknown_contact_without_began_are_dropped() {
    let mut normalizer = Normalizer::default();
    let events = submit(
        &mut normalizer,
        &[
            pencil(9, Moved, 0.0, 0.0, 0.0),
            pencil(9, Ended, 0.1, 0.0, 0.0),
        ],
    );
    assert!(events.is_empty());
    assert_eq!(normalizer.stats().unknown_contact, 2);
}

#[test]
fn rule6_a_newer_generation_cancels_older_contacts_and_drops_their_samples() {
    let mut normalizer = Normalizer::default();
    normalizer
        .submit(&[pencil(1, Began, 0.0, 0.0, 0.0)], 1)
        .unwrap();
    normalizer.drain();
    normalizer
        .submit(
            &[
                pencil(1, Moved, 0.1, 1.0, 0.0),
                pencil(2, Began, 0.1, 5.0, 0.0),
            ],
            2,
        )
        .unwrap();
    let events = normalizer.drain();
    assert_eq!(
        events[0],
        Normalized::Cancel {
            contact: 1,
            reason: CancelReason::Generation
        }
    );
    assert_eq!(contacts(&events), [(2, Began, 5.0)]);
    normalizer
        .submit(&[pencil(1, Ended, 0.2, 1.0, 0.0)], 2)
        .unwrap();
    assert!(
        normalizer.drain().is_empty(),
        "the retired contact's end is dropped"
    );
}

#[test]
fn rule6_a_reused_id_begins_a_new_contact_after_its_cancellation() {
    let mut normalizer = Normalizer::default();
    normalizer
        .submit(&[pencil(1, Began, 0.0, 0.0, 0.0)], 1)
        .unwrap();
    normalizer.cancel_all(CancelReason::System);
    normalizer.drain();
    // The cancelled contact's terminal never arrives; the host reuses its id.
    let events = submit_at(&mut normalizer, 1, &[pencil(1, Began, 0.5, 3.0, 0.0)]);
    assert_eq!(contacts(&events), [(1, Began, 3.0)]);
    let events = submit_at(&mut normalizer, 1, &[pencil(1, Ended, 0.6, 4.0, 0.0)]);
    assert_eq!(contacts(&events), [(1, Ended, 4.0)]);
    assert_eq!(normalizer.live_contacts(), 0);
}

fn submit_at(normalizer: &mut Normalizer, generation: u64, samples: &[Sample]) -> Vec<Normalized> {
    normalizer
        .submit(samples, generation)
        .expect("bounded batch");
    normalizer.drain()
}

#[test]
fn rule6_an_end_in_an_older_generation_batch_becomes_a_cancellation() {
    let mut normalizer = Normalizer::default();
    normalizer
        .submit(&[pencil(1, Began, 0.0, 0.0, 0.0)], 4)
        .unwrap();
    normalizer.drain();
    normalizer
        .submit(&[pencil(1, Ended, 0.1, 9.0, 0.0)], 3)
        .unwrap();
    assert_eq!(
        normalizer.drain(),
        [Normalized::Cancel {
            contact: 1,
            reason: CancelReason::Generation
        }]
    );
}

#[test]
fn rule7_an_oversized_batch_is_refused() {
    let mut normalizer = Normalizer::default();
    let batch = vec![pencil(1, Moved, 0.0, 0.0, 0.0); MAX_BATCH + 1];
    assert_eq!(normalizer.submit(&batch, 0), Err(SubmitError));
}

#[test]
fn rule7_at_most_16_contacts_are_live() {
    let mut normalizer = Normalizer::default();
    let began: Vec<_> = (0..20)
        .map(|c| finger(c, Began, 0.0, c as f64, 0.0))
        .collect();
    let events = submit(&mut normalizer, &began);
    assert_eq!(contacts(&events).len(), MAX_CONTACTS);
    assert_eq!(normalizer.stats().refused_contacts, 4);
    assert_eq!(normalizer.live_contacts(), MAX_CONTACTS);
}

#[test]
fn rule7_overflow_merges_moves_and_keeps_began_and_ended() {
    let mut normalizer = Normalizer::default();
    normalizer
        .submit(&[pencil(1, Began, 0.0, 0.0, 0.0)], 0)
        .unwrap();
    let mut overflowed = false;
    let mut t = 0.0;
    for _ in 0..5 {
        let moves: Vec<_> = (0..MAX_BATCH)
            .map(|_| {
                t += 1.0;
                pencil(1, Moved, t, t, 0.0)
            })
            .collect();
        overflowed |= normalizer.submit(&moves, 0).unwrap();
    }
    overflowed |= normalizer
        .submit(&[pencil(1, Ended, t + 1.0, t, 0.0)], 0)
        .unwrap();
    assert!(overflowed);
    let events = normalizer.drain();
    assert!(events.len() <= MAX_PENDING);
    let phases: Vec<_> = contacts(&events).iter().map(|c| c.1).collect();
    assert_eq!(phases.first(), Some(&Began));
    assert_eq!(phases.last(), Some(&Ended));
    // The newest movement survives the merge.
    assert_eq!(contacts(&events)[events.len() - 2].2, t);
    assert!(normalizer.stats().merged_moves > 0);
}

#[test]
fn missing_optional_capabilities_stay_unavailable() {
    let raw = RawSample {
        contact_id: 1,
        kind: 1,
        phase: 0,
        present: mask::LOCATION,
        timestamp_s: 0.0,
        x_pt: 1.0,
        y_pt: 2.0,
        force: 0.7,
        altitude_rad: 1.0,
        ..RawSample::default()
    };
    let sample = Sample::from_raw(&raw).expect("valid");
    assert_eq!(sample.pose, Pose::default());
    assert_eq!(sample.estimation_index, None);
    assert_eq!(sample.position, Some(p(1.0, 2.0)));
    assert!(Sample::from_raw(&RawSample { kind: 9, ..raw }).is_none());
    assert!(Sample::from_raw(&RawSample {
        x_pt: f64::NAN,
        ..raw
    })
    .is_none());
    assert!(Sample::from_raw(&RawSample {
        y_pt: -1.0e308,
        ..raw
    })
    .is_none());
}
