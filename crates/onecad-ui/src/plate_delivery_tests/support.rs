use super::*;
use onecad_scene::{Camera, Projection, Viewport};

pub(super) struct Harness {
    pub app: App,
    pub context: egui::Context,
    pub frame: eframe::Frame,
}

impl Harness {
    pub async fn new(root: &std::path::Path) -> Self {
        let (mut app, context) = create_app(root).await;
        let mut frame = eframe::Frame::_new_kittest();
        pump_until(&mut app, &context, &mut frame, |app| {
            app.doc.display.is_some()
        })
        .await;
        let original = app
            .doc
            .display
            .as_ref()
            .unwrap()
            .context
            .runtime_session
            .clone();
        app.document_request(Request::New);
        pump_until(&mut app, &context, &mut frame, |app| {
            !app.doc.documents.running
        })
        .await;
        assert_ne!(
            app.doc.display.as_ref().unwrap().context.runtime_session,
            original
        );
        assert!(
            app.doc
                .display
                .as_ref()
                .unwrap()
                .projection
                .features
                .is_empty()
        );
        app.new_sketch();
        let mut harness = Self {
            app,
            context,
            frame,
        };
        harness
            .wait(|app| {
                app.doc
                    .sketch
                    .session
                    .as_ref()
                    .is_some_and(|session| session.phase == SketchPhase::Active)
            })
            .await;
        harness
    }

    pub async fn wait(&mut self, predicate: impl Fn(&App) -> bool) {
        pump_until(&mut self.app, &self.context, &mut self.frame, predicate).await;
    }

    pub fn capture(&mut self) -> ViewSnapshot {
        let session = self.app.doc.sketch.session.as_ref().unwrap();
        let center = session.plane.to_world(Vec2::new(45.0, 25.0).unwrap());
        self.app.view_number += 1;
        let view = ViewSnapshot::new(
            session.document.scene,
            self.app.view_number,
            Camera {
                target: DVec3::new(center.x, center.y, center.z),
                offset: DVec3::new(0.0, 0.0, 150.0),
                projection: Projection::Orthographic,
                ..Default::default()
            },
            Viewport {
                width: 900.0,
                height: 560.0,
                scale_factor: 1.0,
            },
        )
        .unwrap();
        self.app.sketch_dispatch(SketchAction::AcceptView {
            key: session.key.clone(),
            document: session.document.clone(),
            capture: view.identity(),
        });
        assert_eq!(
            self.app
                .doc
                .sketch
                .session
                .as_ref()
                .unwrap()
                .accepted_capture,
            Some(view.identity())
        );
        view
    }

    pub fn select(&mut self, targets: Vec<SketchTarget>) {
        self.app
            .sketch_dispatch(SketchAction::SetTool(SketchTool::Select));
        let view = self.capture();
        let session = self.app.doc.sketch.session.as_ref().unwrap();
        self.app.sketch_dispatch(SketchAction::Select {
            key: session.key.clone(),
            document: session.document.clone(),
            capture: view.identity(),
            targets: targets.clone(),
            mode: SelectionMode::Replace,
        });
        assert_eq!(
            self.app.doc.sketch.session.as_ref().unwrap().selection,
            targets
        );
    }

    fn click(&mut self, view: &ViewSnapshot, xy: [f64; 2], ids: PrimitiveIds) {
        let session = self.app.doc.sketch.session.as_ref().unwrap();
        self.app.sketch_dispatch(SketchAction::Click {
            point: CapturedSketchPoint {
                key: session.key.clone(),
                capture: view.identity(),
                point: Vec2::new(xy[0], xy[1]).unwrap(),
            },
            ids,
        });
    }

    pub async fn primitive(
        &mut self,
        tool: SketchTool,
        first: [f64; 2],
        second: [f64; 2],
    ) -> PrimitiveIds {
        self.app.sketch_dispatch(SketchAction::SetTool(tool));
        let view = self.capture();
        self.click(&view, first, ids());
        let created = ids();
        self.click(&view, second, created.clone());
        let expected = match tool {
            SketchTool::Rectangle => created.entities[7],
            SketchTool::Line => created.entities[2],
            _ => created.entities[1],
        };
        self.wait(|app| {
            app.doc.sketch.session.as_ref().is_some_and(|session| {
                session.phase == SketchPhase::Active
                    && session.authoritative.contains_entity(expected)
            })
        })
        .await;
        self.app
            .sketch_dispatch(SketchAction::SetTool(SketchTool::Select));
        created
    }

    pub async fn relation(
        &mut self,
        relation: Relation,
        targets: Vec<SketchTarget>,
        value: Option<&str>,
    ) -> ConstraintId {
        self.select(targets);
        let before: std::collections::BTreeSet<_> = self
            .sketch()
            .constraints()
            .iter()
            .map(Constraint::id)
            .collect();
        assert!(
            constraint_authoring::applicability(&self.app, relation).is_ok(),
            "{}: {}",
            relation.label(),
            self.app.doc.status
        );
        constraint_authoring::activate_relation(&mut self.app, &self.context, relation);
        if let Some(value) = value {
            assert!(constraint_authoring::editor_open(&self.context));
            self.modal(value);
        }
        self.wait(|app| {
            app.doc.sketch.session.as_ref().is_some_and(|session| {
                session.phase == SketchPhase::Active
                    && session.authoritative.constraints().len() == before.len() + 1
            })
        })
        .await;
        self.sketch()
            .constraints()
            .iter()
            .find(|constraint| !before.contains(&constraint.id()))
            .unwrap()
            .id()
    }

    pub fn modal(&mut self, text: &str) {
        modal_frame(&mut self.app, &self.context, vec![]);
        let command = egui::Modifiers {
            command: true,
            mac_cmd: true,
            ..Default::default()
        };
        modal_frame(
            &mut self.app,
            &self.context,
            vec![
                egui::Event::ModifiersChanged(command),
                key(egui::Key::A, command),
                egui::Event::Text(text.into()),
            ],
        );
        modal_frame(
            &mut self.app,
            &self.context,
            vec![
                egui::Event::ModifiersChanged(egui::Modifiers::NONE),
                key(egui::Key::Enter, egui::Modifiers::NONE),
            ],
        );
    }

    pub fn sketch(&self) -> &Sketch {
        &self.app.doc.sketch.session.as_ref().unwrap().authoritative
    }

    pub async fn edit_width(&mut self, id: ConstraintId, width: &str) {
        constraint_authoring::edit_dimension(&self.app, &self.context, id);
        assert!(constraint_authoring::editor_open(&self.context));
        self.modal(width);
        let width: f64 = width.parse().unwrap();
        self.wait(|app| {
            app.doc.sketch.session.as_ref().is_some_and(|session| {
                session.phase == SketchPhase::Active
                    && session
                        .authoritative
                        .get_constraint(id)
                        .and_then(Constraint::value)
                        .is_some_and(|value| (value.value - width).abs() < 1e-8)
            })
        })
        .await;
    }

    pub async fn sketch_history(&mut self, direction: HistoryDirection) {
        let previous = self
            .app
            .doc
            .sketch
            .session
            .as_ref()
            .unwrap()
            .document
            .revision;
        self.app.sketch_dispatch(SketchAction::History(direction));
        self.wait(|app| {
            app.doc.sketch.session.as_ref().is_some_and(|session| {
                session.phase == SketchPhase::Active && session.document.revision > previous
            })
        })
        .await;
    }

    pub async fn finish(&mut self) {
        self.app.sketch_dispatch(SketchAction::Finish);
        self.wait(|app| !app.sketch_active()).await;
    }
}

pub(super) fn ids() -> PrimitiveIds {
    PrimitiveIds {
        entities: std::array::from_fn(|_| EntityId::new()),
        constraints: std::array::from_fn(|_| ConstraintId::new()),
    }
}

pub(super) fn point(id: EntityId) -> SketchTarget {
    SketchTarget::Point {
        entity: id,
        role: CurvePosition::Arbitrary,
    }
}
pub(super) fn entity(id: EntityId) -> SketchTarget {
    SketchTarget::Entity(id)
}

fn key(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers,
    }
}

fn modal_frame(app: &mut App, context: &egui::Context, events: Vec<egui::Event>) {
    let mut output = context.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(900.0, 560.0),
            )),
            events,
            ..Default::default()
        },
        |_| constraint_authoring::show(app, context),
    );
    output.textures_delta.clear();
}
