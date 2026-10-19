use super::{
    catalog::{CATALOG, find},
    state::{DetailTab, ExtensionsUi, ManagerTab, RowState, Sheet, Status, mock_clock},
    *,
};

#[test]
fn initial_counts_match_design() {
    let x = ExtensionsUi::default();
    assert_eq!(x.counts(), (6, 3, 1));
    assert_eq!(x.status("cnc-prep"), Status::Installed);
    assert_eq!(x.status("print-prep"), Status::Update("1.3.0"));
    assert_eq!(x.status("kicad-bridge"), Status::Available);
    assert_eq!(x.status("wood-joinery"), Status::Available);
}

#[test]
fn filter_by_search_category_and_source() {
    let mut x = ExtensionsUi::default();
    assert_eq!(x.visible().len(), CATALOG.len());
    x.search = " MILL ".into();
    assert_eq!(x.visible(), vec![0]);
    x.search = "bridge".into();
    assert_eq!(x.visible(), vec![2]);
    x.search.clear();
    x.category = 4;
    assert_eq!(x.visible(), vec![2, 5]);
    x.category = 6;
    assert!(x.visible().is_empty());
    x.category = 0;
    x.source = 2;
    assert_eq!(x.visible(), vec![2, 3]);
    x.source = 3;
    assert!(x.visible().is_empty());
}

#[test]
fn install_flow_adds_row_and_toast_text() {
    let mut x = ExtensionsUi::default();
    x.request_install("engineering-plastics");
    assert_eq!(x.sheet, Some(Sheet::Install("engineering-plastics")));
    let toast = x.confirm_install().expect("toast");
    assert_eq!(toast, "Engineering Plastics added · 42 materials");
    assert!(x.sheet.is_none());
    assert_eq!(x.status("engineering-plastics"), Status::Installed);
    assert_eq!(x.counts().0, 7);
    // Existing removed row is replaced, not duplicated.
    x.request_install("kicad-bridge");
    x.confirm_install();
    assert_eq!(x.status("kicad-bridge"), Status::Installed);
    assert_eq!(x.counts().0, 7);
    x.show_toast("t".into(), 10.0);
    assert_eq!(x.toast.as_ref().map(|t| t.until), Some(13.0));
}

#[test]
fn update_applies_version_and_clears_badge() {
    let mut x = ExtensionsUi::default();
    x.request_update("print-prep");
    assert_eq!(x.sheet, Some(Sheet::Update("print-prep")));
    x.confirm_update();
    let row = x.row("print-prep").expect("row");
    assert_eq!((row.version, row.update), ("1.3.0", None));
    assert_eq!(x.counts().2, 0);
    assert_eq!(x.status("print-prep"), Status::Installed);
    x.request_update("cnc-prep");
    assert!(x.sheet.is_none());
}

#[test]
fn restart_cancel_uninstall_and_close() {
    let mut x = ExtensionsUi::default();
    x.restart("topology-check");
    let row = x.row("topology-check").expect("row");
    assert_eq!(row.state, RowState::Normal);
    assert!(row.enabled);
    assert_eq!(x.counts().1, 4);
    x.cancel_install("wood-joinery");
    assert_eq!(
        x.row("wood-joinery").map(|r| r.state),
        Some(RowState::Cancelled)
    );
    assert_eq!(x.counts().0, 5);
    x.uninstall("kicad-bridge");
    assert!(x.row("kicad-bridge").is_none());
    x.request_install("iges-interchange");
    x.close_sheet();
    assert!(x.sheet.is_none());
    assert_eq!(x.status("iges-interchange"), Status::Available);
}

#[test]
fn settings_reset_and_developer_clock() {
    let mut x = ExtensionsUi::default();
    x.settings.relief = 2;
    x.settings.diameter = "3.00".into();
    x.reset_settings();
    assert_eq!(x.settings, Default::default());
    assert_eq!(mock_clock(x.reloaded_at), "12:04:31");
    x.reload_developer(29);
    assert_eq!(mock_clock(x.reloaded_at), "12:05:00");
}

fn headless(page: StartPage, mut x: ExtensionsUi, dark: bool) {
    let ctx = egui::Context::default();
    onecad_widgets::fonts::install(&ctx);
    if let Ok(t) = onecad_widgets::theme::load(dark) {
        onecad_widgets::theme::install(&ctx, t, dark);
    }
    let raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1000.0, 700.0),
        )),
        ..Default::default()
    };
    for _ in 0..2 {
        ctx.run_ui(raw.clone(), |ui| {
            render(ui, &mut x, page);
        })
        .textures_delta
        .clear();
    }
}

#[test]
fn every_page_and_sheet_renders_in_both_themes() {
    for dark in [false, true] {
        let mut x = ExtensionsUi::default();
        headless(StartPage::Extensions, x.clone(), dark);
        for tab in [ManagerTab::Installed, ManagerTab::Updates] {
            x.tab = tab;
            x.developer = true;
            headless(StartPage::Extensions, x.clone(), dark);
        }
        for ext in &CATALOG {
            for tab in [
                DetailTab::Overview,
                DetailTab::Settings,
                DetailTab::Changelog,
            ] {
                x.detail = ext.id;
                x.detail_tab = tab;
                headless(StartPage::ExtensionDetail, x.clone(), dark);
            }
        }
        for sheet in [
            Sheet::Install("kicad-bridge"),
            Sheet::Install("engineering-plastics"),
            Sheet::Update("print-prep"),
        ] {
            x.sheet = Some(sheet);
            headless(StartPage::Extensions, x.clone(), dark);
        }
        x.sheet = None;
        x.show_toast("CNC Prep added".into(), 0.0);
        headless(StartPage::Extensions, x, dark);
    }
    assert!(find("cnc-prep").is_some());
}
