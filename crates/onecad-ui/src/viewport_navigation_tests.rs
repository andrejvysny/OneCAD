use super::model_selection_delivery_tests::{boxed, click, labels, targets};
use super::point_center_delivery_tests::support::Host;

fn orbit_frame(host: &mut Host, mut events: Vec<egui::Event>) {
    events.insert(0, egui::Event::ModifiersChanged(egui::Modifiers::SHIFT));
    host.ui(events);
}

fn secondary(position: egui::Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos: position,
        button: egui::PointerButton::Secondary,
        pressed,
        modifiers: egui::Modifiers::SHIFT,
    }
}

#[tokio::test]
#[ignore = "requires production worker and Metal; synthetic navigation is not physical acceptance"]
async fn orbit_skips_hover_picks_retains_selected_targets_and_refreshes_after_release() {
    let root = tempfile::tempdir().unwrap();
    let mut host = boxed(root.path()).await;
    let target = targets(&host, onecad_scene::PickKind::Face).remove(0);
    click(&mut host, target.0, egui::Modifiers::NONE);
    let selected = labels(&host);
    assert_eq!(selected, [target.1]);
    host.redraw();
    orbit_frame(&mut host, vec![secondary(target.0, true)]);
    let mut position = target.0 + egui::vec2(12.0, 0.0);
    orbit_frame(&mut host, vec![egui::Event::PointerMoved(position)]);
    assert!(
        host.app
            .doc
            .navigation
            .active(host.context.input(|input| input.time))
    );
    let picks = host.app.doc.topology.hover_picks;
    let camera = host.app.doc.camera;
    for _ in 0..8 {
        position += egui::vec2(2.0, 1.0);
        orbit_frame(&mut host, vec![egui::Event::PointerMoved(position)]);
        assert_eq!(
            host.app.doc.topology.hover_picks, picks,
            "orbit must not query topology"
        );
        assert_eq!(
            labels(&host),
            selected,
            "navigation preserves selected topology"
        );
    }
    assert_ne!(
        host.app.doc.camera.offset, camera.offset,
        "the captured secondary drag orbits the camera"
    );
    orbit_frame(&mut host, vec![secondary(position, false)]);
    assert!(
        !host
            .app
            .doc
            .navigation
            .active(host.context.input(|input| input.time))
    );
    assert!(
        host.app.doc.topology.hover_picks > picks,
        "eligible hover resumes on release"
    );
    assert_eq!(labels(&host), selected);
    host.redraw();
    for _ in 0..200 {
        host.ui(vec![]);
        if host.app.doc.topology.settled() && !host.app.draw_dirty {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(
        host.app.doc.topology.uploaded >= 1,
        "selected highlights are rebuilt after transient hover clears"
    );
}
