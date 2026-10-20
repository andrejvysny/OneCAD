use super::*;

pub(crate) struct Host {
    pub(crate) app: App,
    pub(crate) context: egui::Context,
    pub(crate) frame: eframe::Frame,
    pub(crate) size: egui::Vec2,
    pub(crate) widgets: Vec<(String, egui::Rect)>,
}

impl Host {
    pub(crate) async fn new(root: &std::path::Path) -> Self {
        let (mut app, context) = create_app(root).await;
        context.enable_accesskit();
        let mut frame = eframe::Frame::_new_kittest();
        pump_until(&mut app, &context, &mut frame, |app| app.display.is_some()).await;
        app.new_sketch();
        pump_until(&mut app, &context, &mut frame, active).await;
        app.ui.start_open = false;
        app.ui.explorer_open = false;
        app.ui.inspector_open = false;
        app.ui.sketch_snap = false;
        app.fit_pending = false;
        let plane = app.sketch.session.as_ref().unwrap().plane;
        app.camera = Camera {
            target: DVec3::new(plane.origin.x, plane.origin.y, plane.origin.z),
            offset: DVec3::new(plane.normal.x, plane.normal.y, plane.normal.z) * 120.0,
            projection: Projection::Orthographic,
            ..Default::default()
        };
        let mut host = Self {
            app,
            context,
            frame,
            size: egui::vec2(1280.0, 820.0),
            widgets: vec![],
        };
        host.redraw();
        host
    }

    pub(crate) fn ui(&mut self, events: Vec<egui::Event>) {
        self.ui_focused(events, true);
    }

    pub(crate) fn ui_focused(&mut self, events: Vec<egui::Event>, focused: bool) {
        let mut raw = egui::RawInput {
            screen_rect: Some(self.screen()),
            events,
            focused,
            ..Default::default()
        };
        crate::input::route(&mut self.app, &self.context, &mut raw);
        let mut output = self.context.run_ui(raw, |ui| {
            eframe::App::ui(&mut self.app, ui, &mut self.frame)
        });
        self.widgets = output
            .platform_output
            .accesskit_update
            .as_ref()
            .unwrap()
            .nodes
            .iter()
            .filter_map(|(_, node)| {
                Some((
                    node.label().or_else(|| node.value())?.to_owned(),
                    node.bounds()?,
                ))
            })
            .map(|(label, rect)| {
                (
                    label,
                    egui::Rect::from_min_max(
                        egui::pos2(rect.x0 as f32, rect.y0 as f32),
                        egui::pos2(rect.x1 as f32, rect.y1 as f32),
                    ),
                )
            })
            .collect();
        output.textures_delta.clear();
    }

    pub(crate) fn screen(&self) -> egui::Rect {
        egui::Rect::from_min_size(egui::Pos2::ZERO, self.size)
    }

    pub(crate) fn redraw(&mut self) {
        self.app.draw_dirty = true;
        for _ in 0..200 {
            self.ui(vec![]);
            if !self.app.draw_dirty && self.app.view.is_some() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        panic!("native redraw failed: {}", self.app.status);
    }

    pub(crate) fn rect(&self, label: &str) -> egui::Rect {
        self.widgets
            .iter()
            .find(|(name, _)| name == label)
            .map(|(_, rect)| *rect)
            .unwrap_or_else(|| {
                panic!(
                    "missing control {label}; controls={:?}",
                    self.widgets
                        .iter()
                        .map(|(name, _)| name)
                        .collect::<Vec<_>>()
                )
            })
    }

    pub(crate) fn click(&mut self, position: egui::Pos2) {
        self.ui(vec![
            egui::Event::PointerMoved(position),
            button(position, true),
        ]);
        self.ui(vec![button(position, false)]);
    }

    pub(crate) fn control(&mut self, label: &str) {
        for _ in 0..20 {
            self.ui(vec![]);
            if self.widgets.iter().any(|(name, _)| name == label) {
                break;
            }
        }
        self.click(self.rect(label).center());
    }

    pub(crate) fn xy(&mut self, x: f64, y: f64) {
        self.redraw();
        self.click(self.project(x, y));
    }

    pub(crate) fn project(&self, x: f64, y: f64) -> egui::Pos2 {
        let plane = self.app.sketch.session.as_ref().unwrap().plane;
        let world = plane.to_world(Vec2::new(x, y).unwrap());
        let screen = self
            .app
            .view
            .unwrap()
            .project(DVec3::new(world.x, world.y, world.z))
            .unwrap();
        let viewport = crate::layout::chrome(self.screen(), &self.app.ui).viewport;
        viewport.min + egui::vec2(screen.logical.x as f32, screen.logical.y as f32)
    }

    pub(crate) fn replace(&mut self, text: &str) {
        let command = egui::Modifiers {
            command: true,
            mac_cmd: true,
            ..Default::default()
        };
        self.ui(vec![
            egui::Event::ModifiersChanged(command),
            key(egui::Key::A, command),
            egui::Event::Text(text.into()),
        ]);
        self.ui(vec![egui::Event::ModifiersChanged(egui::Modifiers::NONE)]);
    }

    pub(crate) fn numeric(&mut self, first: &str, second: &str) {
        self.ui(vec![key(egui::Key::Tab, egui::Modifiers::NONE)]);
        assert!(crate::sketch_numeric::editor_open(&self.context));
        let accepted = self.sketch().clone();
        self.replace(first);
        self.ui(vec![key(egui::Key::Tab, egui::Modifiers::NONE)]);
        self.replace(second);
        assert_eq!(
            self.sketch(),
            &accepted,
            "editing fields must not mutate accepted geometry"
        );
        self.ui(vec![key(egui::Key::Enter, egui::Modifiers::NONE)]);
        assert!(!crate::sketch_numeric::editor_open(&self.context));
    }

    pub(crate) async fn count(&mut self, count: usize) {
        pump_until(&mut self.app, &self.context, &mut self.frame, |app| {
            active(app)
                && app
                    .sketch
                    .session
                    .as_ref()
                    .unwrap()
                    .authoritative
                    .entities()
                    .len()
                    == count
        })
        .await;
    }

    pub(crate) fn sketch(&self) -> &Sketch {
        &self.app.sketch.session.as_ref().unwrap().authoritative
    }

    pub(crate) async fn history(&mut self, redo: bool, count: usize) {
        let command = egui::Modifiers {
            command: true,
            mac_cmd: true,
            shift: redo,
            ..Default::default()
        };
        self.ui(vec![
            egui::Event::ModifiersChanged(command),
            key(egui::Key::Z, command),
        ]);
        self.count(count).await;
        self.ui(vec![egui::Event::ModifiersChanged(egui::Modifiers::NONE)]);
    }

    pub(crate) async fn stop(mut self) {
        self.app.stop_discard();
        pump_until(&mut self.app, &self.context, &mut self.frame, |app| {
            app.may_close
        })
        .await;
    }
}

pub(crate) fn active(app: &App) -> bool {
    app.sketch
        .session
        .as_ref()
        .is_some_and(|session| session.phase == SketchPhase::Active)
}

pub(crate) fn key(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
    egui::Event::Key {
        key,
        modifiers,
        physical_key: None,
        pressed: true,
        repeat: false,
    }
}

pub(crate) fn button(pos: egui::Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos,
        pressed,
        button: egui::PointerButton::Primary,
        modifiers: egui::Modifiers::NONE,
    }
}

pub(crate) fn near(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < 1e-5, "{actual} != {expected}");
}
