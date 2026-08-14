// ─────────────────────────────────────────────────────────────────────────────
// Case runners
// ─────────────────────────────────────────────────────────────────────────────

/// A core [`Sketch`] from a case's machine-readable entity list.
///
/// The corpus states geometry the way the C++ oracle drew it — a Line by its two
/// endpoints, an Arc by centre/radius/endpoints — while the core models a Line by
/// two POINT ENTITIES and an Arc by centre plus sweep ANGLES. This is that
/// translation, and nothing more: no constraint is invented, because region
/// detection closes loops on coincident COORDINATES, and adding solver constraints
/// here would silently change what the oracle is being asked.
fn sketch_from_corpus(label: &str, entities: &[Value]) -> Result<Sketch, String> {
    let mut sketch = Sketch::on_world_plane(SketchId(stable_uuid(label)), label, WorldPlane::XY);
    let mut next: u128 = 0;
    let mut fresh = |sketch: &mut Sketch, at: Vec2| -> Result<EntityId, String> {
        next += 1;
        let id = EntityId(Uuid::from_u128(next | (1 << 96)));
        sketch
            .add_entity(SketchEntity::point(id, at, false, false))
            .map_err(|e| format!("point {id:?}: {e:?}"))?;
        Ok(id)
    };
    let vec2 = |value: &Value, key: &str| -> Result<Vec2, String> {
        let arr = value
            .get(key)
            .and_then(Value::as_array)
            .ok_or(format!("missing {key}"))?;
        let (x, y) = (
            arr.first()
                .and_then(Value::as_f64)
                .ok_or(format!("{key}[0]"))?,
            arr.get(1)
                .and_then(Value::as_f64)
                .ok_or(format!("{key}[1]"))?,
        );
        Ok(Vec2::new_unchecked(x, y))
    };
    let number = |value: &Value, key: &str| -> Result<f64, String> {
        value
            .get(key)
            .and_then(Value::as_f64)
            .ok_or(format!("missing {key}"))
    };

    for entity in entities {
        let id = EntityId(stable_uuid(&format!(
            "{label}:{}",
            entity.get("id").and_then(Value::as_str).unwrap_or("?")
        )));
        match entity.get("type").and_then(Value::as_str) {
            Some("Line") => {
                let start = fresh(&mut sketch, vec2(entity, "p0")?)?;
                let end = fresh(&mut sketch, vec2(entity, "p1")?)?;
                sketch
                    .add_entity(SketchEntity::line(id, start, end, false))
                    .map_err(|e| format!("line: {e:?}"))?;
            }
            Some("Arc") => {
                let centre_at = vec2(entity, "center")?;
                let centre = fresh(&mut sketch, centre_at)?;
                let radius = number(entity, "radius")?;
                // The corpus gives the arc's ENDPOINTS; the core wants sweep angles.
                let angle = |point: Vec2| (point.y - centre_at.y).atan2(point.x - centre_at.x);
                let arc = SketchEntity::arc(
                    id,
                    centre,
                    radius,
                    angle(vec2(entity, "start")?),
                    angle(vec2(entity, "end")?),
                    false,
                )
                .ok_or("arc rejected (non-finite radius/angles)")?;
                sketch.add_entity(arc).map_err(|e| format!("arc: {e:?}"))?;
            }
            Some("Circle") => {
                let centre = fresh(&mut sketch, vec2(entity, "center")?)?;
                let circle = SketchEntity::circle(id, centre, number(entity, "radius")?, false)
                    .ok_or("circle rejected (non-finite radius)")?;
                sketch
                    .add_entity(circle)
                    .map_err(|e| format!("circle: {e:?}"))?;
            }
            Some("Ellipse") => {
                let centre = fresh(&mut sketch, vec2(entity, "center")?)?;
                let ellipse = SketchEntity::ellipse(
                    id,
                    centre,
                    number(entity, "majorR")?,
                    number(entity, "minorR")?,
                    number(entity, "rotation").unwrap_or(0.0),
                    false,
                )
                .ok_or("ellipse rejected (non-finite radii)")?;
                sketch
                    .add_entity(ellipse)
                    .map_err(|e| format!("ellipse: {e:?}"))?;
            }
            other => return Err(format!("entity type {other:?} has no corpus translation")),
        }
    }
    Ok(sketch)
}

/// `a_sketch_extrude_blind` — the whole expected block, not just the last volume:
/// the sketch step's region count, the extrude step's body events, solid count and
/// volume, and the derived face count.
async fn run_sketch_extrude_blind(
    wm: &WorkerManager,
    case: &CorpusCase,
    epoch: WorkerEpoch,
) -> Result<(), String> {
    let expected: SketchExtrudeExpected = serde_json::from_value(case.expected.clone())
        .map_err(|e| format!("{} expected schema: {e}", case.id))?;
    // Labels are consumed while BUILDING the timeline; case a addresses its bodies
    // through the plan's own per-step results, so none are needed afterwards.
    let (tl, _labels) = timeline_of(case, None)?;
    let (prepared, events) = run_plan(wm, plan_request(&tl, 0xA, epoch)).await?;
    if prepared.stopped_reason != StoppedReason::Completed {
        return Err(format!("plan stopped: {:?}", prepared.stopped_reason));
    }
    accept(wm, 0xA, epoch).await?;

    for step_expected in &expected.per_step {
        let index = step_expected.step_index;
        let what = format!("{} step {index}", case.id);

        // The sketch step's region oracle, read from the solver lane the frontend uses.
        if let Some(regions) = step_expected.regions {
            // Region detection lives on the SOLVER lane, which the plan's Sketch op
            // does not populate — the plan materializes the sketch for the geometry
            // lane. So the same entities are upserted here, exactly as the frontend
            // does before a profile pick, and the oracle is read from that.
            let entities = case.op_script[index]
                .params
                .get("entities")
                .and_then(Value::as_array)
                .ok_or("sketch step carries no entities")?;
            let sketch = sketch_from_corpus(&case.op_script[index].op_id, entities)
                .map_err(|e| format!("{what}: {e}"))?;
            wm.sketch_upsert(&sketch)
                .await
                .map_err(|e| format!("{what}: SketchUpsert failed: {e}"))?;
            let detected = wm
                .sketch_regions(&sketch.id.to_string())
                .await
                .map_err(|e| format!("{what}: SketchRegions failed: {e}"))?;
            if detected.regions.len() as u64 != regions {
                return Err(format!(
                    "{what}: expected {regions} region(s), detected {}",
                    detected.regions.len()
                ));
            }
        }

        check_lifecycle_claims(
            &step_expected.body_events,
            events.get(&index).map_or(&[], Vec::as_slice),
            &what,
        )?;

        if let Some(solids) = step_expected.solids {
            let produced = prepared
                .per_step
                .iter()
                .find(|s| s.step_index == index)
                .map_or(0, |s| s.body_ids.len());
            if produced as u64 != solids {
                return Err(format!(
                    "{what}: expected {solids} solid(s), got {produced}"
                ));
            }
        }

        if let Some(volume) = &step_expected.volume_mm3 {
            let body = prepared
                .per_step
                .iter()
                .find(|s| s.step_index == index)
                .and_then(|s| s.body_ids.first().copied())
                .ok_or_else(|| format!("{what}: no body to measure"))?;
            check_measured_volume(
                volume.value(),
                step_expected.volume_tol.unwrap_or(1e-6),
                volume_of(wm, body).await?,
                &what,
            )?;

            // `faceCount` is recorded as derived-not-asserted (the C++ test checks
            // volume + validity only). Verifying it here is exactly what the case
            // asks for — "verify against worker output" — and it is the assertion
            // that would catch a prism built from the wrong profile at the right
            // volume.
            let faces = expected.face_count.value;
            let actual = topology_of(wm, body).await?;
            if let Some(solids) = step_expected.solids {
                if u64::from(actual.solid_count) != solids {
                    return Err(format!(
                        "{what}: expected {solids} BRep solid(s), got {}",
                        actual.solid_count
                    ));
                }
            }
            if u64::from(actual.face_count) != faces {
                return Err(format!(
                    "{what}: expected {faces} BRep faces, got {}",
                    actual.face_count
                ));
            }
        }
    }
    Ok(())
}

/// `b_extrude_throughall_symmetric_twodir` — three INDEPENDENT scenarios (the C++
/// oracles are separate documents), each its own plan against a reset session.
async fn run_extrude_end_conditions(
    wm: &WorkerManager,
    case: &CorpusCase,
    mut epoch: WorkerEpoch,
) -> Result<WorkerEpoch, String> {
    let expected: EndConditionExpected = serde_json::from_value(case.expected.clone())
        .map_err(|e| format!("{} expected schema: {e}", case.id))?;
    let scenarios: [(&str, &VolumeClaim, u128, &str); 3] = [
        (
            "throughAllCut",
            &expected.through_all_cut,
            0xB1,
            "op_base_box",
        ),
        ("twoDirection", &expected.two_direction, 0xB2, "op_twodir"),
        ("symmetric", &expected.symmetric, 0xB3, "op_symmetric"),
    ];
    for (scenario, claim, job, body_op) in scenarios {
        epoch = reset_document(wm, scenario, epoch).await?;
        let (tl, mut labels) = timeline_of(case, Some(scenario))?;
        let (prepared, events) = run_plan(wm, plan_request(&tl, job, epoch)).await?;
        if prepared.stopped_reason != StoppedReason::Completed {
            return Err(format!(
                "{scenario}: plan stopped: {:?}",
                prepared.stopped_reason
            ));
        }
        accept(wm, job, epoch).await?;

        let what = format!("{} / {scenario}", case.id);
        let body = body_of_record(RecordId(labels.uuid(body_op)));

        // The through-all case claims BOTH volumes: the base box before the cut and
        // the result after it. Only the second is measurable on the final head, so
        // the first is asserted through the step that produced it.
        if let Some(after) = &claim.after_cut {
            check_lifecycle_claims(
                &claim.body_events,
                events.values().last().map_or(&[], Vec::as_slice),
                &what,
            )?;
            check_volume_claim(
                after,
                volume_of(wm, body).await?,
                &format!("{what} afterCut"),
            )?;
        } else {
            let last_step = events.keys().copied().next_back().unwrap_or(0);
            if !claim.body_events.is_empty() {
                check_lifecycle_claims(
                    &claim.body_events,
                    events.get(&last_step).map_or(&[], Vec::as_slice),
                    &what,
                )?;
            }
            check_volume_claim(claim, volume_of(wm, body).await?, &what)?;
        }
    }
    Ok(epoch)
}

/// `i_multiregion_loop_detection` — the loop-detector oracle, run through the
/// solver lane's `SketchRegions` (the same verb the extrude profile pick uses).
async fn run_loop_detection(wm: &WorkerManager, case: &CorpusCase) -> Result<(), String> {
    let expected: RegionsExpected = serde_json::from_value(case.expected.clone())
        .map_err(|e| format!("{} expected schema: {e}", case.id))?;
    for op in &case.op_script {
        let recorded = match op.op_id.as_str() {
            "regions_rect" => &expected.rectangle,
            "regions_square_hole" => &expected.square_with_hole,
            "regions_arc_chord" => &expected.arc_and_chord,
            "regions_ellipse" => &expected.ellipse,
            _ => return Err(format!("no typed expected block for {}", op.op_id)),
        };
        let claim = recorded.new_stack.as_deref().unwrap_or(recorded);
        let what = format!("{} / {}", case.id, op.op_id);
        let entities = op
            .sketch
            .as_ref()
            .and_then(|s| s.get("entities"))
            .and_then(Value::as_array)
            .ok_or_else(|| format!("{what}: no machine-readable entities"))?;

        let sketch = sketch_from_corpus(&op.op_id, entities).map_err(|e| format!("{what}: {e}"))?;
        wm.sketch_upsert(&sketch)
            .await
            .map_err(|e| format!("{what}: SketchUpsert failed: {e}"))?;
        let detected = wm
            .sketch_regions(&sketch.id.to_string())
            .await
            .map_err(|e| format!("{what}: SketchRegions failed: {e}"))?;

        // `regions` is either an exact count or the ">=1" the C++ test asserts.
        match &claim.regions {
            CountClaim::Exact(want) => {
                if detected.regions.len() as u64 != *want {
                    return Err(format!(
                        "{what}: expected {want} region(s), detected {}",
                        detected.regions.len()
                    ));
                }
            }
            CountClaim::AtLeastOne(token) if token == ">=1" => {
                if detected.regions.is_empty() {
                    return Err(format!(
                        "{what}: expected at least one region, detected none"
                    ));
                }
            }
            CountClaim::AtLeastOne(token) => {
                return Err(format!("{what}: unreadable regions claim {token}"));
            }
        }

        if let Some(holes) = claim.holes {
            let detected_holes: usize = detected.regions.iter().map(|r| r.holes.len()).sum();
            if detected_holes as u64 != holes {
                return Err(format!(
                    "{what}: expected {holes} hole(s), detected {detected_holes}"
                ));
            }
        }
    }
    Ok(())
}
