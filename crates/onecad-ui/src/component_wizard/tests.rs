//! The Create Component form: id/version rules, step gating, configuration editing and the
//! request lifecycle, all without a backend.
use super::{
    Op, Reply,
    model::{Landed, Phase, WizardState},
    validate::{self, *},
};
use crate::component_instances::Scope;
use onecad_app::{
    dto::{LibraryComponentDto, VariableDto},
    library::AttachmentSpecInput,
};
use onecad_core::{document::record::ComponentParamValue as Value, ids::BodyId};
use std::collections::BTreeMap;

fn scope() -> Scope {
    Scope {
        document_id: "doc".into(),
        runtime_session: "one".into(),
    }
}

fn variable(name: &str, value: f64) -> VariableDto {
    VariableDto {
        id: name.into(),
        name: name.into(),
        value,
        expr: None,
        resolved_value: value,
        dimension: "length".into(),
        error: None,
    }
}

fn variables() -> Vec<VariableDto> {
    vec![variable("width", 80.0), variable("thickness", 4.0)]
}

fn wizard() -> WizardState {
    WizardState::new(scope(), BodyId::new(), "Plate".into())
}

#[test]
fn ids_follow_the_library_rules() {
    assert_eq!(id_error("user.bracket"), None);
    assert_eq!(id_error("acme.parts.m6-screw_a"), None);
    for bad in [
        "",
        "bracket",
        "User.bracket",
        "a..b",
        ".a.b",
        "a.b.",
        "a/b.c",
        "a b.c",
        "../x.y",
    ] {
        assert!(id_error(bad).is_some(), "{bad:?} must be refused");
    }
}

#[test]
fn versions_are_semver_triples() {
    assert_eq!(version_error("1.0.0"), None);
    assert_eq!(version_error("12.3.45"), None);
    for bad in [
        "",
        "1",
        "1.0",
        "1.0.0.0",
        "1.0.x",
        "v1.0.0",
        "1..0",
        "1.0.0-rc1",
    ] {
        assert!(version_error(bad).is_some(), "{bad:?} must be refused");
    }
}

#[test]
fn the_suggested_id_is_a_valid_slug_of_the_name() {
    assert_eq!(suggest_id("Mounting Bracket"), "user.mounting_bracket");
    assert_eq!(suggest_id("  M6 -- screw! "), "user.m6_screw");
    assert_eq!(suggest_id("???"), "user.component");
    for name in ["Plate", "Ünï cödé 2", "a.b"] {
        assert_eq!(id_error(&suggest_id(name)), None, "{name}");
    }
}

#[test]
fn configuration_and_attachment_names_are_checked_and_unique() {
    assert_eq!(configuration_name_error("Small", ["Heavy"]), None);
    assert!(configuration_name_error("", []).is_some());
    assert!(configuration_name_error("a/b", []).is_some());
    assert!(
        configuration_name_error("Small", ["Small"])
            .unwrap()
            .contains("Another")
    );
    assert_eq!(attachment_name_error("top", ["base"]), None);
    assert!(attachment_name_error("top face", []).is_some());
    assert!(attachment_name_error("top", ["top"]).is_some());
    assert!(attachment_name_error(&"x".repeat(33), []).is_some());
}

#[test]
fn numbers_must_be_finite_and_plain() {
    assert_eq!(validate::number(" 12.5 "), Ok(12.5));
    assert!(validate::number("").is_err());
    assert!(validate::number("inf").is_err());
    assert!(
        validate::number("=width")
            .unwrap_err()
            .contains("expressions")
    );
}

#[test]
fn details_gate_the_second_step() {
    let mut w = wizard();
    assert!(w.can_advance(), "the source step needs nothing more");
    w.advance();
    assert_eq!(w.step, 1);
    w.name = "  ".into();
    assert!(!w.can_advance());
    assert_eq!(w.blocker(1).as_deref(), Some("Enter a name"));
    w.name = "Plate".into();
    w.id = "plate".into();
    assert!(w.blocker(1).unwrap().contains("namespaced"));
    w.id = "user.plate".into();
    w.version = "1.0".into();
    assert!(w.blocker(1).unwrap().contains("1.0.0"));
    w.version = "1.0.0".into();
    assert_eq!(w.blocker(1), None);
    w.advance();
    assert_eq!(w.step, 2);
    w.back();
    assert_eq!(w.step, 1);
}

#[test]
fn the_id_follows_the_name_until_typed() {
    let mut w = wizard();
    w.name = "Heavy Plate".into();
    w.follow_name();
    assert_eq!(w.id, "user.heavy_plate");
    w.id = "acme.plate".into();
    w.id_edited = true;
    w.name = "Other".into();
    w.follow_name();
    assert_eq!(w.id, "acme.plate");
}

#[test]
fn parametric_needs_an_exposed_variable_and_valid_configurations() {
    let vars = variables();
    let mut w = wizard();
    w.parametric = true;
    assert!(w.blocker(2).unwrap().contains("Expose at least one"));
    w.set_exposed(&vars, "width", true);
    w.set_exposed(&vars, "thickness", true);
    assert_eq!(w.blocker(2), None, "configurations are optional");
    w.add_config(&vars);
    w.add_config(&vars);
    assert_eq!(w.configs[0].name, "Config 1");
    assert_eq!(w.configs[1].name, "Config 2");
    assert_eq!(w.configs[0].values["width"], "80");
    w.rename_config(0, "Small".into());
    w.rename_config(1, "Small".into());
    assert!(w.blocker(2).unwrap().contains("Another configuration"));
    w.rename_config(1, "Heavy".into());
    w.configs[1].values.insert("width".into(), "abc".into());
    assert!(w.blocker(2).unwrap().contains("Heavy / width"));
    w.configs[1].values.insert("width".into(), "120".into());
    assert_eq!(w.blocker(2), None);
    // A static save never asks for parameters.
    w.parametric = false;
    w.exposed.clear();
    assert_eq!(w.blocker(2), None);
}

#[test]
fn configuration_cells_follow_the_exposed_set_and_the_default_follows_its_name() {
    let vars = variables();
    let mut w = wizard();
    w.parametric = true;
    w.set_exposed(&vars, "width", true);
    w.add_config(&vars);
    w.rename_config(0, "Small".into());
    w.default_config = Some("Small".into());
    w.set_exposed(&vars, "thickness", true);
    assert_eq!(w.configs[0].values["thickness"], "4");
    w.set_exposed(&vars, "width", false);
    assert!(!w.configs[0].values.contains_key("width"));
    w.rename_config(0, "Tiny".into());
    assert_eq!(w.default_config.as_deref(), Some("Tiny"));
    w.remove_config(0);
    assert!(w.configs.is_empty() && w.default_config.is_none());
}

fn ready_parametric() -> WizardState {
    let vars = variables();
    let mut w = wizard();
    w.parametric = true;
    w.id = "user.plate".into();
    w.set_exposed(&vars, "width", true);
    w.set_exposed(&vars, "thickness", true);
    w.add_config(&vars);
    w.add_config(&vars);
    w.rename_config(0, "Small".into());
    w.rename_config(1, "Heavy".into());
    w.configs[1].values.insert("thickness".into(), "8".into());
    w.default_config = Some("Small".into());
    w.category = "plates, brackets ,".into();
    w.tags = "flat".into();
    w
}

#[test]
fn the_parametric_spec_carries_parameters_configurations_and_the_default() {
    let w = ready_parametric();
    let spec = w.spec(&variables()).expect("valid");
    assert!(!spec.static_save);
    assert_eq!(
        (spec.id.as_str(), spec.version.as_str()),
        ("user.plate", "1.0.0")
    );
    assert_eq!(spec.category, ["plates", "brackets"]);
    assert_eq!(spec.parameters["width"].value, 80.0);
    assert_eq!(spec.parameters["thickness"].key, "thickness");
    assert_eq!(
        spec.configurations["Heavy"]["thickness"],
        Value::Number(8.0)
    );
    assert_eq!(spec.configurations["Small"]["width"], Value::Number(80.0));
    assert_eq!(spec.default_configuration.as_deref(), Some("Small"));
}

#[test]
fn a_static_spec_has_no_parameters_even_if_some_were_exposed_earlier() {
    let mut w = ready_parametric();
    w.parametric = false;
    let spec = w.spec(&variables()).expect("valid");
    assert!(spec.static_save);
    assert!(spec.parameters.is_empty() && spec.configurations.is_empty());
    assert!(spec.default_configuration.is_none());
}

#[test]
fn the_spec_is_refused_for_a_vanished_variable_or_an_unknown_designation_key() {
    let w = ready_parametric();
    let only_width = vec![variable("width", 80.0)];
    assert!(
        w.spec(&only_width)
            .unwrap_err()
            .contains("`thickness` no longer exists")
    );
    let mut w = ready_parametric();
    w.designation = "Plate {width}x{depth}".into();
    assert!(w.spec(&variables()).unwrap_err().contains("{depth}"));
    w.designation = "Plate {width}x{thickness}".into();
    assert_eq!(
        w.spec(&variables()).unwrap().designation.as_deref(),
        Some("Plate {width}x{thickness}")
    );
    w.designation = "Plate {width".into();
    assert!(w.spec(&variables()).unwrap_err().contains("unclosed"));
}

fn input(on: &str) -> AttachmentSpecInput {
    AttachmentSpecInput {
        on: on.into(),
        accepts: vec!["plane".into()],
        frame: None,
    }
}

#[test]
fn a_pick_names_the_attachment_and_the_reply_adds_a_row() {
    let mut w = wizard();
    w.new_attachment = "top face".into();
    assert!(w.begin_pick().is_err());
    w.new_attachment = "top".into();
    w.begin_pick().expect("valid name");
    assert_eq!(w.phase, Phase::Picking);
    w.resolve_pick("f:2".into(), 7);
    assert_eq!(w.phase, Phase::Resolving);
    let queued = w.take_outbox();
    let Op::Pick {
        name,
        topo_key,
        snapshot_id,
        body_id,
    } = &queued[0].op
    else {
        panic!("expected a pick");
    };
    assert_eq!(
        (name.as_str(), topo_key.as_str(), *snapshot_id),
        ("top", "f:2", 7)
    );
    assert_eq!(*body_id, w.body.to_string());
    let landed = w.accept(
        Some(&scope()),
        queued[0].sequence,
        Ok(Reply::Picked(Box::new(input("face:top")))),
    );
    assert_eq!(landed, Landed::Picked);
    assert_eq!(w.attachments[0].name, "top");
    assert_eq!(w.new_attachment, "attach_2");
    assert_eq!(w.phase, Phase::Editing);
    // Names must stay unique.
    w.new_attachment = "top".into();
    assert!(w.begin_pick().unwrap_err().contains("Another attachment"));
    w.rename_attachment(0, "seat").expect("renamed");
    assert_eq!(w.attachments[0].input.on, "face:seat");
    w.remove_attachment(0);
    assert!(w.attachments.is_empty());
}

#[test]
fn a_refused_pick_shows_the_service_reason() {
    let mut w = wizard();
    w.begin_pick().unwrap();
    w.resolve_pick("e:1".into(), 1);
    let sequence = w.take_outbox()[0].sequence;
    let reason =
        "attachmentFromPick: attachments need a planar face, a cylinder or a circular edge";
    assert_eq!(
        w.accept(Some(&scope()), sequence, Err(reason.into())),
        Landed::Refused
    );
    assert_eq!(
        w.error.as_deref(),
        Some("attachments need a planar face, a cylinder or a circular edge")
    );
    assert_eq!(w.phase, Phase::Editing);
}

#[test]
fn replies_for_another_document_or_an_old_request_are_dropped() {
    let moved = Scope {
        runtime_session: "two".into(),
        ..scope()
    };
    let reply = || Ok(Reply::Picked(Box::new(input("face:a"))));
    let mut w = wizard();
    w.begin_pick().unwrap();
    w.resolve_pick("f:2".into(), 1);
    let sequence = w.take_outbox()[0].sequence;
    assert_eq!(
        w.accept(Some(&scope()), sequence + 1, reply()),
        Landed::Dropped
    );
    assert_eq!(w.accept(Some(&scope()), sequence, reply()), Landed::Picked);
    assert_eq!(
        w.accept(Some(&scope()), sequence, reply()),
        Landed::Dropped,
        "applied once"
    );
    // A pick answered after the document moved on is not applied.
    let mut w = wizard();
    w.begin_pick().unwrap();
    w.resolve_pick("f:2".into(), 1);
    let sequence = w.take_outbox()[0].sequence;
    assert_eq!(w.accept(Some(&moved), sequence, reply()), Landed::Dropped);
    assert!(w.attachments.is_empty());
}

#[test]
fn a_save_in_flight_reports_even_after_the_document_changed() {
    let vars = variables();
    let moved = Scope {
        runtime_session: "two".into(),
        ..scope()
    };
    let mut w = ready_parametric();
    w.request_save(&vars);
    let sequence = w.take_outbox()[0].sequence;
    let landed = w.accept(
        Some(&moved),
        sequence,
        Ok(Reply::Saved(Box::new(saved_dto()))),
    );
    assert!(
        matches!(landed, Landed::Saved(_)),
        "the package is on disk; say so"
    );
    let mut w = ready_parametric();
    w.request_save(&vars);
    let sequence = w.take_outbox()[0].sequence;
    assert_eq!(
        w.accept(None, sequence, Err("disk full".into())),
        Landed::Refused
    );
    assert_eq!(w.error.as_deref(), Some("disk full"));
    assert_eq!(w.phase, Phase::Editing);
}

#[test]
fn a_reply_of_the_wrong_kind_settles_the_phase_instead_of_waiting_forever() {
    let vars = variables();
    let mut w = ready_parametric();
    w.request_save(&vars);
    let sequence = w.take_outbox()[0].sequence;
    let wrong = Ok(Reply::Picked(Box::new(input("face:a"))));
    assert_eq!(w.accept(Some(&scope()), sequence, wrong), Landed::Refused);
    assert_eq!(w.phase, Phase::Editing);
    assert!(w.error.as_deref().unwrap().contains("unexpected reply"));
    let mut w = wizard();
    w.begin_pick().unwrap();
    w.resolve_pick("f:2".into(), 1);
    let sequence = w.take_outbox()[0].sequence;
    let wrong = Ok(Reply::Saved(Box::new(saved_dto())));
    assert_eq!(w.accept(Some(&scope()), sequence, wrong), Landed::Refused);
    assert_eq!(w.phase, Phase::Editing);
    assert!(!w.busy());
}

fn saved_dto() -> LibraryComponentDto {
    crate::library_catalog::testing::component("user.plate", "1.0.0", BTreeMap::new())
}

#[test]
fn save_outcomes_are_honest() {
    let vars = variables();
    let mut w = ready_parametric();
    w.step = 3;
    w.request_save(&vars);
    assert_eq!(w.phase, Phase::Saving);
    let queued = w.take_outbox();
    let Op::Save {
        spec, union_solids, ..
    } = &queued[0].op
    else {
        panic!("expected a save");
    };
    assert!(!*union_solids && !spec.static_save);
    let multi = format!(
        "saveAsComponent: {}: a component must resolve to exactly one solid",
        onecad_app::library::MULTI_SOLID_REFUSAL
    );
    assert_eq!(
        w.accept(Some(&scope()), queued[0].sequence, Err(multi)),
        Landed::Refused
    );
    assert!(w.union_offer && w.error.as_deref().unwrap().contains("Union solids"));
    w.union_solids = true;
    w.request_save(&vars);
    let queued = w.take_outbox();
    assert!(matches!(
        &queued[0].op,
        Op::Save {
            union_solids: true,
            ..
        }
    ));
    let taken =
        "user.plate@1.0.0 already exists — re-saving a component is a version bump".to_owned();
    assert_eq!(
        w.accept(Some(&scope()), queued[0].sequence, Err(taken)),
        Landed::Refused
    );
    assert_eq!(
        w.error.as_deref(),
        Some("That id@version already exists — bump the version")
    );
    assert!(!w.union_offer);
    w.version = "1.0.1".into();
    w.request_save(&vars);
    let queued = w.take_outbox();
    let landed = w.accept(
        Some(&scope()),
        queued[0].sequence,
        Ok(Reply::Saved(Box::new(saved_dto()))),
    );
    assert!(matches!(landed, Landed::Saved(_)));
    assert!(matches!(w.phase, Phase::Saved(_)));
}

#[test]
fn saving_with_a_blocked_form_sends_nothing() {
    let mut w = ready_parametric();
    w.version = "x".into();
    w.request_save(&variables());
    assert!(w.take_outbox().is_empty());
    assert_eq!(w.phase, Phase::Editing);
    assert!(w.error.as_deref().unwrap().contains("1.0.0"));
}

fn render(w: &mut WizardState, dark: bool) -> String {
    let ctx = egui::Context::default();
    onecad_widgets::fonts::install(&ctx);
    if let Ok(t) = onecad_widgets::theme::load(dark) {
        onecad_widgets::theme::install(&ctx, t, dark);
    }
    let raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1100.0, 800.0),
        )),
        ..Default::default()
    };
    let vars = variables();
    let mut text = String::new();
    for _ in 0..2 {
        let mut out = ctx.run_ui(raw.clone(), |ui| {
            if w.phase == Phase::Picking {
                super::chrome::pick_banner(ui.ctx(), w);
            } else {
                super::chrome::modal(ui.ctx(), w, &vars);
            }
        });
        text.clear();
        fn walk(shape: &egui::Shape, out: &mut String) {
            match shape {
                egui::Shape::Text(t) => {
                    out.push_str(t.galley.text());
                    out.push('\n');
                }
                egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
                _ => {}
            }
        }
        out.shapes.iter().for_each(|s| walk(&s.shape, &mut text));
        out.textures_delta.clear();
    }
    text
}

#[test]
fn every_step_and_phase_renders_in_both_themes() {
    for dark in [false, true] {
        let mut w = ready_parametric();
        w.attachments.push(super::model::AttachmentRow {
            name: "top".into(),
            input: input("face:top"),
        });
        for step in 0..4 {
            w.step = step;
            let page = render(&mut w, dark);
            assert!(
                page.contains("Create component") || page.contains("Source"),
                "{page}"
            );
        }
        w.step = 2;
        let page = render(&mut w, dark);
        for expected in [
            "width",
            "thickness",
            "Small",
            "Heavy",
            "Add configuration",
            "Default configuration",
        ] {
            assert!(page.contains(expected), "{expected} in {page}");
        }
        w.step = 3;
        let page = render(&mut w, dark);
        assert!(
            page.contains("Pick face/edge") && page.contains("face:top"),
            "{page}"
        );
        assert!(page.contains("Save component"), "{page}");
        w.step = 1;
        w.id = "Bad".into();
        assert!(render(&mut w, dark).contains("namespaced"));
        w.step = 3;
        w.phase = Phase::Picking;
        assert!(render(&mut w, dark).contains("Click a planar face"));
        w.phase = Phase::Saved(super::model::Saved {
            id: "user.plate".into(),
            version: "1.0.0".into(),
            name: "Plate".into(),
        });
        let page = render(&mut w, dark);
        assert!(
            page.contains("Saved Plate 1.0.0 to the library") && page.contains("Open in library"),
            "{page}"
        );
        w.phase = Phase::Editing;
        w.parametric = false;
        w.step = 2;
        assert!(render(&mut w, dark).contains("static component exposes no parameters"));
    }
}
