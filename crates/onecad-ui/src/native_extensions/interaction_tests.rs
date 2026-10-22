//! Exercise actual production controls through egui input, without a CAD renderer.
use super::*;
use crate::ui_state::StartPage;
use egui::{Context, Event, Pos2, RawInput, Rect, Shape, vec2};
use onecad_extension_host::PackageState;

fn fixture() -> State {
    let manifest: onecad_extension_api::Manifest = serde_json::from_str(include_str!(
        "../../../../addons/bambu-studio/manifest.json"
    ))
    .unwrap();
    let commands = manifest
        .contributions
        .commands
        .iter()
        .cloned()
        .map(|command| RegisteredCommand {
            extension_id: manifest.id.clone(),
            command,
            enabled: false,
        })
        .collect();
    State {
        loaded: true,
        attempted: true,
        commands,
        packages: vec![PackageSummary {
            manifest,
            fingerprint: "reviewed-package-fingerprint".into(),
            state: PackageState::Disabled,
            path: "/tmp/onecad-test-addon".into(),
        }],
        ..Default::default()
    }
}

fn context() -> Context {
    let context = Context::default();
    onecad_widgets::fonts::install(&context);
    onecad_widgets::theme::install(&context, onecad_widgets::theme::load(false).unwrap(), false);
    context
}

fn frame(
    context: &Context,
    state: &mut State,
    events: Vec<Event>,
) -> (ViewAction, Vec<egui::epaint::ClippedShape>) {
    frame_on(
        context,
        state,
        StartPage::Extensions,
        vec2(1040.0, 756.0),
        events,
    )
}

fn frame_on(
    context: &Context,
    state: &mut State,
    page: StartPage,
    size: egui::Vec2,
    events: Vec<Event>,
) -> (ViewAction, Vec<egui::epaint::ClippedShape>) {
    let input = RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
        focused: true,
        events,
        ..Default::default()
    };
    let mut action = ViewAction::default();
    let mut output = context.run_ui(input, |ui| {
        egui::CentralPanel::default()
            .frame(egui::Frame::new().inner_margin(32))
            .show(ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    action = render(
                        ui,
                        state,
                        page,
                        PresentationContext {
                            idle: true,
                            document: true,
                            selection: false,
                        },
                    );
                });
            });
    });
    output.textures_delta.clear();
    (action, output.shapes)
}

fn label_center(shapes: &[egui::epaint::ClippedShape], label: &str) -> Pos2 {
    fn find(shape: &Shape, label: &str) -> Option<Pos2> {
        match shape {
            Shape::Text(text) if text.galley.text() == label => {
                Some(text.pos + text.galley.size() * 0.5)
            }
            Shape::Vec(shapes) => shapes.iter().find_map(|shape| find(shape, label)),
            _ => None,
        }
    }
    shapes
        .iter()
        .find_map(|shape| find(&shape.shape, label))
        .unwrap_or_else(|| panic!("Production control {label:?} was not rendered"))
}

fn click(context: &Context, state: &mut State, label: &str) -> ViewAction {
    let mut shapes = Vec::new();
    for _ in 0..3 {
        shapes = frame(context, state, Vec::new()).1;
    }
    let position = label_center(&shapes, label);
    let pointer = |pressed| {
        vec![
            Event::PointerMoved(position),
            Event::PointerButton {
                pos: position,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            },
        ]
    };
    frame(context, state, pointer(true));
    frame(context, state, pointer(false)).0
}

#[test]
fn installed_details_control_navigates_to_the_actual_package() {
    let mut state = fixture();
    let action = click(&context(), &mut state, "Details");
    assert_eq!(action.page, Some(StartPage::ExtensionDetail));
    assert_eq!(
        state.detail.as_deref(),
        Some(state.packages[0].manifest.id.as_str())
    );
    assert!(action.request.is_none());
    assert!(action.command.is_none());
}

#[test]
fn installed_row_controls_stay_next_to_the_title_and_footer_stays_visible() {
    for width in [1040.0, 660.0] {
        let context = context();
        let mut state = fixture();
        let mut shapes = Vec::new();
        for _ in 0..3 {
            shapes = frame_on(
                &context,
                &mut state,
                StartPage::Extensions,
                vec2(width, 756.0),
                Vec::new(),
            )
            .1;
        }
        let name = label_center(&shapes, &state.packages[0].manifest.name);
        let details = label_center(&shapes, "Details");
        assert!(
            (name.y - details.y).abs() < 40.0,
            "Row controls stretched vertically: {name:?} {details:?}"
        );
        assert!(label_center(&shapes, "Install from folder…").y < 700.0);
    }
}

#[test]
fn detail_header_keeps_tabs_and_overview_visible_at_normal_and_narrow_widths() {
    for width in [1040.0, 660.0] {
        let context = context();
        let mut state = fixture();
        state.detail = Some(state.packages[0].manifest.id.clone());
        let mut shapes = Vec::new();
        for _ in 0..3 {
            shapes = frame_on(
                &context,
                &mut state,
                StartPage::ExtensionDetail,
                vec2(width, 756.0),
                Vec::new(),
            )
            .1;
        }
        assert!(
            label_center(&shapes, "Overview").y < 250.0,
            "Detail header consumed viewport height"
        );
        assert!(
            label_center(&shapes, "Run command").y < 700.0,
            "Overview was pushed below viewport"
        );
    }
}

#[test]
fn discover_long_description_and_title_stay_inside_the_real_card() {
    fn card_rect(shape: &Shape) -> Option<Rect> {
        match shape {
            Shape::Rect(rect) if (rect.rect.height() - 180.0).abs() < 0.1 => Some(rect.rect),
            Shape::Vec(shapes) => shapes.iter().find_map(card_rect),
            _ => None,
        }
    }
    fn text_rect(shape: &Shape, label: &str) -> Option<Rect> {
        match shape {
            Shape::Text(text) if text.galley.text() == label => {
                Some(Rect::from_min_size(text.pos, text.galley.size()))
            }
            Shape::Vec(shapes) => shapes.iter().find_map(|shape| text_rect(shape, label)),
            _ => None,
        }
    }
    for width in [1040.0, 660.0] {
        let context = context();
        let mut state = fixture();
        state.tab = ManagerTab::Discover;
        state.packages[0].manifest.name =
            "Long extension title with several descriptive words ".repeat(3);
        state.packages[0].manifest.description =
            "Export selected bodies with document attributes. ".repeat(20);
        let mut shapes = Vec::new();
        for _ in 0..3 {
            shapes = frame_on(
                &context,
                &mut state,
                StartPage::Extensions,
                vec2(width, 756.0),
                Vec::new(),
            )
            .1;
        }
        let card = shapes
            .iter()
            .find_map(|shape| card_rect(&shape.shape))
            .unwrap();
        for label in [
            &state.packages[0].manifest.name,
            &state.packages[0].manifest.description,
        ] {
            let text = shapes
                .iter()
                .find_map(|shape| text_rect(&shape.shape, label))
                .unwrap();
            assert!(
                card.contains_rect(text),
                "Card text escaped its bounds: card={card:?} text={text:?}"
            );
            assert!(
                text.bottom() <= card.bottom() - 38.0,
                "Text overlapped the card footer"
            );
        }
    }
}

#[test]
fn consent_allow_returns_exact_grants_without_optimistic_enable() {
    let mut state = fixture();
    let package = state.packages[0].clone();
    state.review = Some(package.clone());
    let action = click(&context(), &mut state, "Allow & enable");
    let Some(Request::SetEnabled {
        id,
        enabled,
        approval: Some(approval),
    }) = action.request
    else {
        panic!("Allow must produce a permission-bearing enable request");
    };
    assert_eq!(id, package.manifest.id);
    assert!(enabled);
    assert_eq!(approval.fingerprint, package.fingerprint);
    assert_eq!(approval.capabilities, package.manifest.capabilities);
    assert_eq!(
        approval.applications,
        package
            .manifest
            .applications
            .iter()
            .map(|app| app.id.clone())
            .collect::<Vec<_>>()
    );
    assert!(!approval.trust_native);
    assert!(state.review.is_none());
    assert_eq!(state.packages[0].state, PackageState::Disabled);
}

#[test]
fn consent_cancel_and_escape_dismiss_without_granting_permissions() {
    for escape in [false, true] {
        let mut state = fixture();
        state.review = Some(state.packages[0].clone());
        let context = context();
        let action = if escape {
            for _ in 0..3 {
                frame(&context, &mut state, Vec::new());
            }
            frame(
                &context,
                &mut state,
                vec![Event::Key {
                    key: egui::Key::Escape,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }],
            )
            .0
        } else {
            click(&context, &mut state, "Cancel")
        };
        assert!(state.review.is_none(), "escape={escape}");
        assert!(action.request.is_none());
        assert_eq!(state.packages[0].state, PackageState::Disabled);
    }
}

#[test]
fn empty_manager_can_recover_application_settings_only_after_confirmation() {
    let mut state = fixture();
    state.packages.clear();
    state.commands.clear();
    let context = context();
    let action = click(&context, &mut state, "Reset application paths…");
    assert!(action.request.is_none());
    assert!(state.reset_confirmation);
    let action = click(&context, &mut state, "Back up & reset");
    assert!(matches!(action.request, Some(Request::ResetApplications)));
    assert!(!state.reset_confirmation);
}
