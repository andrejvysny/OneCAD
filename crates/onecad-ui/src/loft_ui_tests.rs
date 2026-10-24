use super::*;
use onecad_core::ids::{RegionId, SketchId, SnapshotId};
use onecad_interaction::ProfileSelection;
use onecad_scene::{Camera, SceneIdentity, ViewSnapshot, Viewport};

fn context() -> DocumentContext {
    DocumentContext {
        document_id: "loft-doc".into(),
        runtime_session: "loft-runtime".into(),
        revision: 1,
        snapshot_id: SnapshotId(1),
        scene: SceneIdentity {
            session: 1,
            scene: 1,
        },
    }
}

fn reference() -> SketchRegionRef {
    SketchRegionRef {
        sketch: SketchId::new(),
        region: RegionId::new("chosen-region"),
        region_identity_version: Some(3),
        region_anchor: Some([1.25, 2.75]),
        extra: Default::default(),
    }
}

fn pick(profile: SketchRegionRef, context: DocumentContext) -> SelectionItem {
    let capture = ViewSnapshot::new(
        context.scene,
        1,
        Camera::default(),
        Viewport {
            width: 800.0,
            height: 600.0,
            scale_factor: 1.0,
        },
    )
    .unwrap()
    .identity();
    SelectionItem::Profile(Box::new(ProfileSelection {
        profile,
        context,
        capture,
    }))
}

#[test]
fn loft_selection_keeps_authored_order_and_refuses_stale_or_unfinished_profiles() {
    let current = context();
    let profiles = [reference(), reference(), reference()];
    let picks: Vec<_> = profiles
        .iter()
        .rev()
        .cloned()
        .map(|p| pick(p, current.clone()))
        .collect();
    assert_eq!(
        selected_profiles(&picks, &current, |_| true).unwrap(),
        profiles.iter().rev().cloned().collect::<Vec<_>>()
    );
    assert!(selected_profiles(&picks, &current, |_| false).is_err());
    let mut stale = current.clone();
    stale.revision += 1;
    assert!(selected_profiles(&picks, &stale, |_| true).is_err());
    let invalid = [SelectionItem::Sketch(SketchId::new())];
    assert!(selected_profiles(&invalid, &current, |_| true).is_err());
}

fn controls_frame(
    ctx: &egui::Context,
    sections: &mut Vec<SketchRegionRef>,
    index: usize,
    replacement: Option<&SketchRegionRef>,
    events: Vec<egui::Event>,
) -> egui::FullOutput {
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(800.0, 600.0),
            )),
            events,
            ..Default::default()
        },
        |ui| section_controls(ui, sections, index, replacement),
    );
    output.textures_delta.clear();
    output
}

fn activate(
    ctx: &egui::Context,
    sections: &mut Vec<SketchRegionRef>,
    index: usize,
    replacement: Option<&SketchRegionRef>,
    suffix: &str,
) {
    controls_frame(ctx, sections, index, replacement, Vec::new());
    let out = controls_frame(ctx, sections, index, replacement, Vec::new());
    let id = format!("onecad.option.loftSection.{index}.{suffix}");
    let update = out.platform_output.accesskit_update.unwrap();
    let node = update
        .nodes
        .iter()
        .map(|(_, n)| n)
        .find(|n| n.author_id() == Some(id.as_str()))
        .unwrap();
    let rect = node.bounds().unwrap();
    let pos = egui::pos2(
        ((rect.x0 + rect.x1) / 2.0) as f32,
        ((rect.y0 + rect.y1) / 2.0) as f32,
    );
    let events = vec![
        egui::Event::PointerMoved(pos),
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        },
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        },
    ];
    controls_frame(ctx, sections, index, replacement, events);
}

#[test]
fn native_section_buttons_reorder_replace_remove_without_rebuilding_references() {
    let ctx = egui::Context::default();
    onecad_widgets::theme::install(&ctx, onecad_widgets::theme::load(false).unwrap(), false);
    ctx.enable_accesskit();
    let original = [reference(), reference(), reference()];
    let mut profiles = original.to_vec();
    activate(&ctx, &mut profiles, 2, None, "up");
    assert_eq!(
        profiles,
        [
            original[0].clone(),
            original[2].clone(),
            original[1].clone()
        ]
    );
    let replacement = reference();
    activate(&ctx, &mut profiles, 1, Some(&replacement), "replace");
    assert_eq!(profiles[1], replacement);
    activate(&ctx, &mut profiles, 0, None, "remove");
    assert_eq!(profiles, [replacement, original[1].clone()]);
}

fn card_frame(
    ctx: &egui::Context,
    params: &LoftParams,
    events: Vec<egui::Event>,
) -> (
    egui::FullOutput,
    egui::Rect,
    onecad_widgets::components::overlays::FeatureCardOutput,
) {
    use onecad_widgets::components::overlays::{FeatureCard, FeatureCardOutput};
    let labels: Vec<_> = params
        .profiles
        .iter()
        .enumerate()
        .map(|(i, _)| {
            format!(
                "Long named offset sketch section {} with preserved region identity",
                i + 1
            )
        })
        .collect();
    let mut bounds = egui::Rect::NOTHING;
    let mut footer = FeatureCardOutput::default();
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(820.0, 1180.0),
            )),
            events,
            ..Default::default()
        },
        |ui| {
            let card = egui::Area::new(egui::Id::unique("loft-portrait-test"))
                .fixed_pos(egui::pos2(20.0, 20.0))
                .show(ui.ctx(), |ui| {
                    footer = FeatureCard::new("loft", "loft", "Loft")
                        .width(280.0)
                        .max_height(320.0)
                        .show(ui, |rows| {
                            section_fields(rows, params, &labels, &[]);
                        });
                    crate::feature_card::name_footer(ui.ctx(), footer.ok_id, footer.cancel_id);
                });
            bounds = card.response.rect;
        },
    );
    output.textures_delta.clear();
    (output, bounds, footer)
}

fn node_rect(output: &egui::FullOutput, id: &str) -> egui::Rect {
    let update = output.platform_output.accesskit_update.as_ref().unwrap();
    let node = update
        .nodes
        .iter()
        .map(|(_, n)| n)
        .find(|n| n.author_id() == Some(id))
        .unwrap_or_else(|| panic!("missing {id}"));
    let rect = node.bounds().unwrap();
    egui::Rect::from_min_max(
        egui::pos2(rect.x0 as f32, rect.y0 as f32),
        egui::pos2(rect.x1 as f32, rect.y1 as f32),
    )
}

#[test]
fn portrait_feature_card_contains_wrapped_sections_and_keeps_scrolling_footer_reachable() {
    let ctx = egui::Context::default();
    onecad_widgets::theme::install(&ctx, onecad_widgets::theme::load(false).unwrap(), false);
    ctx.enable_accesskit();
    let params = LoftParams {
        profiles: (0..12).map(|_| reference()).collect(),
        is_solid: true,
        is_ruled: false,
        boolean_mode: onecad_core::document::record::BooleanMode::NewBody,
        extra: Default::default(),
    };
    card_frame(&ctx, &params, Vec::new());
    let (initial, card, _) = card_frame(&ctx, &params, Vec::new());
    assert!(card.width() <= 282.0, "{card:?}");
    assert!(card.height() <= 322.0, "{card:?}");
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(820.0, 1180.0));
    assert!(screen.contains_rect(card));
    let update = initial.platform_output.accesskit_update.as_ref().unwrap();
    for (_, node) in &update.nodes {
        if node
            .author_id()
            .is_some_and(|id| id.starts_with("onecad.option.loft"))
            && let Some(bounds) = node.bounds()
        {
            assert!(
                bounds.x0 >= f64::from(card.left()) && bounds.x1 <= f64::from(card.right()),
                "control overflow: {:?} {bounds:?}",
                node.author_id()
            );
        }
    }
    let scroll = vec![
        egui::Event::PointerMoved(card.center()),
        egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Unknown,
            delta: egui::vec2(0.0, -10000.0),
            modifiers: egui::Modifiers::NONE,
        },
    ];
    card_frame(&ctx, &params, scroll);
    let (scrolled, bounds, _) = card_frame(&ctx, &params, Vec::new());
    assert!(bounds.contains_rect(node_rect(&scrolled, "onecad.option.loftSection.11.replace")));
    for id in [crate::automation::OP_CONFIRM, crate::automation::OP_CANCEL] {
        assert!(
            bounds.contains_rect(node_rect(&scrolled, id)),
            "footer {id} clipped"
        );
    }
}
