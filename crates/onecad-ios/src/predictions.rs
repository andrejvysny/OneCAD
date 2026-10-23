//! Predicted ink exists only in the final egui output, never in accepted input.
use onecad_ui::host::PresentationContext;

#[derive(Clone, Debug)]
pub struct Snapshot {
    pub contact: u64,
    pub generation: u64,
    pub context: PresentationContext,
    pub anchor: egui::Pos2,
    pub points: Vec<egui::Pos2>,
}

pub fn append(
    output: &mut egui::FullOutput,
    snapshot: &Snapshot,
    current: &PresentationContext,
    contact: (u64, u64),
    clips: &[egui::Rect],
    accent: egui::Color32,
) {
    if !matches(snapshot, current, contact) || !current.allows_prediction() || clips.len() > 64 {
        return;
    }
    append_shapes(output, snapshot, clips, accent);
}

fn matches(snapshot: &Snapshot, current: &PresentationContext, contact: (u64, u64)) -> bool {
    &snapshot.context == current && (snapshot.contact, snapshot.generation) == contact
}

fn append_shapes(
    output: &mut egui::FullOutput,
    snapshot: &Snapshot,
    clips: &[egui::Rect],
    accent: egui::Color32,
) {
    let points = std::iter::once(snapshot.anchor)
        .chain(
            snapshot
                .points
                .iter()
                .copied()
                .take(crate::input::BATCH_MAX),
        )
        .collect::<Vec<_>>();
    if points.len() < 2 || points.iter().any(|point| !point.is_finite()) {
        return;
    }
    let shape = egui::Shape::line(points, egui::Stroke::new(1.5, accent.gamma_multiply(0.4)));
    output
        .shapes
        .extend(clips.iter().filter(|clip| clip.is_positive()).map(|clip| {
            egui::epaint::ClippedShape {
                clip_rect: *clip,
                shape: shape.clone(),
                transform_after_tessellation: egui::emath::TSTransform::IDENTITY,
            }
        }));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot() -> Snapshot {
        Snapshot {
            contact: 7,
            generation: 3,
            context: Default::default(),
            anchor: egui::pos2(10.0, 20.0),
            points: vec![egui::pos2(30.0, 40.0)],
        }
    }

    #[test]
    fn output_ink_starts_at_confirmed_point_uses_theme_and_actual_clip() {
        let snapshot = snapshot();
        let clip = egui::Rect::from_min_max(egui::pos2(5.0, 5.0), egui::pos2(25.0, 25.0));
        let mut output = egui::FullOutput::default();
        append_shapes(&mut output, &snapshot, &[clip], egui::Color32::BLUE);
        assert_eq!(output.shapes.len(), 1);
        assert_eq!(output.shapes[0].clip_rect, clip);
        let egui::Shape::Path(path) = &output.shapes[0].shape else {
            panic!("prediction path")
        };
        assert_eq!(path.points, vec![snapshot.anchor, snapshot.points[0]]);
        assert_eq!(
            path.stroke.color,
            egui::epaint::ColorMode::Solid(egui::Color32::BLUE.gamma_multiply(0.4))
        );
        assert!(output.platform_output.commands.is_empty());
    }

    #[test]
    fn stale_contact_generation_context_and_home_never_emit_output() {
        let snapshot = snapshot();
        for mismatch in 0..4 {
            let mut context = snapshot.context.clone();
            let contact = match mismatch {
                0 => (8, 3),
                1 => (7, 4),
                _ => (7, 3),
            };
            if mismatch == 2 {
                context.owner.tab += 1;
            }
            assert_eq!(matches(&snapshot, &context, contact), mismatch == 3);
            let mut output = egui::FullOutput::default();
            append(
                &mut output,
                &snapshot,
                &context,
                contact,
                &[egui::Rect::EVERYTHING],
                egui::Color32::BLUE,
            );
            assert!(output.shapes.is_empty());
        }
    }
}
