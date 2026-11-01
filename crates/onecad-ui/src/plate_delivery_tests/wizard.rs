//! Create Component through the production host with the real worker and Metal: expose the
//! plate's variables, add two configurations, pick the top face as an attachment, save, see
//! it in the catalog, refuse a duplicate id@version, then a static save. Synthetic pointer
//! delivery is not physical acceptance.
use super::{
    placement::{click, frame, plate_with_hole, rim_position, until},
    *,
};
use crate::component_wizard::{self, Op, Request};
use crate::ui_state::StartPage;

fn variables(app: &App) -> Vec<onecad_app::dto::VariableDto> {
    app.doc
        .display
        .as_ref()
        .map(|d| d.projection.variables.clone())
        .unwrap_or_default()
}

/// Adds a document variable through the wizard lane's test setup op.
async fn variable(host: &mut Harness, name: &str, value: f64) {
    let scope = crate::component_instances::scope(&host.app).unwrap();
    let request = Request {
        sequence: 1000,
        scope,
        op: Op::Variable {
            name: name.into(),
            value,
        },
    };
    host.app
        .doc
        .bridge
        .submit(Command::Author(request))
        .unwrap();
    let name = name.to_owned();
    until(host, "the variable", |app| {
        variables(app).iter().any(|v| v.name == name)
    })
    .await;
}

fn wizard(app: &App) -> &component_wizard::WizardState {
    app.doc.wizard.as_ref().expect("wizard open")
}

fn wizard_mut(app: &mut App) -> &mut component_wizard::WizardState {
    app.doc.wizard.as_mut().expect("wizard open")
}

async fn open_on_body(host: &mut Harness) {
    let body = host.app.doc.display.as_ref().unwrap().scene.bodies()[0].body_id();
    crate::item_selection::body(
        &mut host.app,
        body,
        onecad_interaction::selection_set::SelectionMode::Replace,
    );
    assert_eq!(component_wizard::availability(&host.app), None);
    component_wizard::open(&mut host.app);
    assert!(host.app.doc.wizard.is_some());
}

fn fill_details(host: &mut Harness, name: &str, id: &str, version: &str, parametric: bool) {
    let vars = variables(&host.app);
    let w = wizard_mut(&mut host.app);
    w.parametric = parametric;
    w.name = name.into();
    w.id = id.into();
    w.id_edited = true;
    w.version = version.into();
    w.category = "plates".into();
    w.tags = "test".into();
    if parametric {
        w.set_exposed(&vars, "width", true);
        w.set_exposed(&vars, "thickness", true);
        w.add_config(&vars);
        w.add_config(&vars);
        w.rename_config(0, "Small".into());
        w.rename_config(1, "Heavy".into());
        w.configs[1].values.insert("width".into(), "120".into());
        w.configs[1].values.insert("thickness".into(), "8".into());
        w.default_config = Some("Small".into());
    }
    w.step = 3;
}

async fn pick_top_face(host: &mut Harness, rim: DVec3) {
    // Esc cancels a pick without clearing anything else.
    wizard_mut(&mut host.app).new_attachment = "top".into();
    wizard_mut(&mut host.app).begin_pick().unwrap();
    let escape = egui::Event::Key {
        key: egui::Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    };
    frame(host, vec![escape], egui::Modifiers::NONE);
    assert!(!component_wizard::picking(&host.app));
    wizard_mut(&mut host.app).new_attachment = "top".into();
    wizard_mut(&mut host.app).begin_pick().unwrap();
    let face = DVec3::new(rim.x, rim.y + 42.0, rim.z);
    let position = rim_position(host, face);
    until(host, "the pick banner", |app| {
        component_wizard::picking(app)
    })
    .await;
    click(host, position, egui::Modifiers::NONE);
    until(host, "the attachment", |app| {
        app.doc
            .wizard
            .as_ref()
            .is_some_and(|w| !w.attachments.is_empty())
    })
    .await;
}

async fn save(host: &mut Harness) {
    let vars = variables(&host.app);
    wizard_mut(&mut host.app).request_save(&vars);
    until(host, "the save to settle", |app| {
        app.doc.wizard.as_ref().is_some_and(|w| !w.busy())
    })
    .await;
}

#[tokio::test]
#[ignore = "requires production worker and Metal; synthetic pointer delivery is not physical acceptance"]
async fn create_component_saves_a_parametric_package_refuses_a_duplicate_and_saves_static() {
    assert!(std::env::var_os("ONECAD_LIBRARY_ROOT").is_none());
    let area = tempfile::tempdir().unwrap();
    let mut host = Harness::new(area.path()).await;
    let rim = plate_with_hole(&mut host).await;
    variable(&mut host, "width", 80.0).await;
    variable(&mut host, "thickness", 4.0).await;
    host.app.ui.start_open = false;

    // The pill and the library page agree on why nothing is selected.
    assert_eq!(
        component_wizard::availability(&host.app),
        Some("Select the body to make into a component")
    );
    open_on_body(&mut host).await;
    assert!(host.app.busy(), "the wizard owns the editor");

    // Parametric: both variables exposed, two configurations, the top face attached.
    fill_details(&mut host, "Test plate", "user.test_plate", "1.0.0", true);
    pick_top_face(&mut host, rim).await;
    assert!(
        wizard(&host.app).attachments[0]
            .input
            .on
            .starts_with("face:top")
    );
    assert_eq!(wizard(&host.app).attachments[0].input.accepts, ["plane"]);
    save(&mut host).await;
    let w = wizard(&host.app);
    assert!(
        matches!(w.phase, component_wizard::Phase::Saved(_)),
        "{:?} {:?}",
        w.phase,
        w.error
    );
    assert_eq!(host.app.doc.status, "Saved Test plate 1.0.0 to the library");

    // The catalog lists it, with its configurations.
    host.app.ui.start_open = true;
    host.app.ui.start_page = StartPage::Library;
    until(&mut host, "the catalog entry", |app| {
        app.ui
            .library
            .catalog
            .results()
            .iter()
            .any(|c| c.id == "user.test_plate")
    })
    .await;
    let entry = host
        .app
        .ui
        .library
        .catalog
        .results()
        .iter()
        .find(|c| c.id == "user.test_plate")
        .unwrap()
        .clone();
    assert_eq!(entry.name, "Test plate");
    assert!(entry.parameters.contains_key("width") && entry.parameters.contains_key("thickness"));
    assert_eq!(
        entry.configurations["Heavy"]["width"].as_float(),
        Some(120.0)
    );
    assert_eq!(entry.default_configuration.as_deref(), Some("Small"));
    assert!(entry.attachments.keys().any(|k| k.starts_with("top")));
    host.app.ui.start_open = false;
    host.app.doc.wizard = None;

    // The same id@version again is refused with the bump hint; the wizard stays open.
    open_on_body(&mut host).await;
    fill_details(&mut host, "Test plate", "user.test_plate", "1.0.0", true);
    save(&mut host).await;
    assert_eq!(
        wizard(&host.app).error.as_deref(),
        Some("That id@version already exists — bump the version")
    );
    // Bumping the version saves.
    wizard_mut(&mut host.app).version = "1.0.1".into();
    save(&mut host).await;
    assert!(matches!(
        wizard(&host.app).phase,
        component_wizard::Phase::Saved(_)
    ));
    host.app.doc.wizard = None;

    // Static: no free parameters, no configurations.
    open_on_body(&mut host).await;
    fill_details(
        &mut host,
        "Static plate",
        "user.static_plate",
        "1.0.0",
        false,
    );
    save(&mut host).await;
    assert!(matches!(
        wizard(&host.app).phase,
        component_wizard::Phase::Saved(_)
    ));
    host.app.ui.start_open = true;
    host.app.ui.start_page = StartPage::Library;
    until(&mut host, "the static entry", |app| {
        app.ui
            .library
            .catalog
            .results()
            .iter()
            .any(|c| c.id == "user.static_plate")
    })
    .await;
    let entry = host
        .app
        .ui
        .library
        .catalog
        .results()
        .iter()
        .find(|c| c.id == "user.static_plate")
        .unwrap();
    assert!(entry.parameters.is_empty(), "{:?}", entry.parameters);
    assert!(entry.configurations.is_empty());
    host.app.stop_discard();
    host.wait(|app| app.may_close).await;
}
