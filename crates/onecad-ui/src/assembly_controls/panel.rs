use super::{Action, submit};
use crate::app::App;
use onecad_app::dto::AssemblyProjectionDto;
use onecad_assembly::{
    joint::{Joint, JointId, JointKind, JointLimits, JointOrigin},
    math::{Frame, Quat, RigidTransform, Vec3d},
    occurrence::{ComponentDefId, Occurrence, OccurrenceId},
};
use onecad_core::edit::AssemblyEdit;

pub(crate) fn panel(app: &mut App, ui: &mut egui::Ui) {
    if app.ui.workbench != crate::workbench::Workbench::Assembly {
        return;
    }
    let Some(display) = &app.doc.display else {
        return;
    };
    let projection = display.assembly.clone();
    let scene = display.scene.identity();
    if app.doc.assembly_controls.draft_scene != Some(scene) {
        let state = &mut app.doc.assembly_controls;
        state.pose_for = None;
        state.origin_picks.clear();
        state.unbound_sides.clear();
        state.draft = state.joint.and_then(|id| {
            projection
                .joints
                .iter()
                .find(|joint| joint.id == id)
                .cloned()
        });
        if state.joint.is_some() && state.draft.is_none() {
            state.joint = None;
        }
        state.draft_scene = Some(scene);
    }
    if let Some(selected) = &app.doc.topology.occurrence {
        let id = selected
            .path
            .0
            .last()
            .map(|part| OccurrenceId(uuid::Uuid::from_u128(part.0)));
        if id.is_some() {
            app.doc.assembly_controls.occurrence = id;
        }
    }
    ui.separator();
    ui.heading("Assembly");
    ui.small("Assembly STEP exports visible posed solids in mm; hierarchy, names, appearance, joints and PMI are omitted.");
    if app.doc.assembly_controls.pending {
        ui.label("Applying assembly change…");
        if app.doc.assembly_controls.cancellation.is_some()
            && ui.button("Cancel joint solve").clicked()
        {
            super::cancel(app);
        }
        return;
    }
    if !app.doc.bridge.valid() {
        ui.label("Scene is stale.");
        if super::recovery_undo_available(app) && ui.button("Undo accepted change").clicked() {
            app.submit(crate::bridge::Command::Undo);
        }
        return;
    }
    definitions(app, ui, &projection);
    super::definition_edit::panel(app, ui, &projection);
    occurrences(app, ui, &projection);
    joints(app, ui, &projection);
}

fn definitions(app: &mut App, ui: &mut egui::Ui, projection: &AssemblyProjectionDto) {
    let state = &mut app.doc.assembly_controls;
    ui.label("Embedded definitions");
    ui.horizontal(|ui| {
        ui.label("Name");
        ui.text_edit_singleline(&mut state.name);
    });
    if ui.button("Copy root as definition").clicked() {
        let name = if state.name.trim().is_empty() {
            "Part".to_string()
        } else {
            state.name.trim().to_string()
        };
        let id = ComponentDefId::new();
        state.definition = Some(id);
        submit(app, Action::Edit(AssemblyEdit::CopyRoot { id, name }));
        return;
    }
    let definitions: Vec<_> = projection
        .definitions
        .iter()
        .filter(|definition| definition.id != projection.root_definition)
        .collect();
    egui::ComboBox::from_id_salt("assembly-definition")
        .selected_text(
            state
                .definition
                .and_then(|id| {
                    definitions
                        .iter()
                        .find(|d| d.id == id.to_string())
                        .map(|d| d.name.as_str())
                })
                .unwrap_or("Select definition"),
        )
        .show_ui(ui, |ui| {
            for definition in definitions {
                if let Ok(id) = uuid::Uuid::parse_str(&definition.id) {
                    ui.selectable_value(
                        &mut state.definition,
                        Some(ComponentDefId(id)),
                        &definition.name,
                    );
                }
            }
        });
    ui.horizontal(|ui| {
        ui.label("Instance name");
        ui.text_edit_singleline(&mut state.occurrence_name);
    });
    if ui
        .add_enabled(
            state.definition.is_some_and(|id| {
                projection.definitions.iter().any(|definition| {
                    definition.id == id.to_string() && definition.id != projection.root_definition
                })
            }),
            egui::Button::new("Insert occurrence"),
        )
        .clicked()
    {
        let Some(def) = state.definition else {
            return;
        };
        let id = OccurrenceId::new();
        state.occurrence = Some(id);
        let name = if state.occurrence_name.trim().is_empty() {
            "Occurrence".into()
        } else {
            state.occurrence_name.trim().into()
        };
        submit(
            app,
            Action::Edit(AssemblyEdit::InsertOccurrence {
                occurrence: Occurrence {
                    id,
                    def,
                    parent: None,
                    local: RigidTransform::IDENTITY,
                    grounded: false,
                    name,
                },
            }),
        );
    }
}

fn occurrences(app: &mut App, ui: &mut egui::Ui, projection: &AssemblyProjectionDto) {
    ui.separator();
    ui.label("Occurrences");
    let state = &mut app.doc.assembly_controls;
    let mut selected_in_list = false;
    egui::ComboBox::from_id_salt("assembly-occurrence")
        .selected_text(
            state
                .occurrence
                .and_then(|id| {
                    projection
                        .occurrences
                        .iter()
                        .find(|o| o.id == id.to_string())
                        .map(|o| o.name.as_str())
                })
                .unwrap_or("Select occurrence"),
        )
        .show_ui(ui, |ui| {
            for occurrence in &projection.occurrences {
                if let Ok(id) = uuid::Uuid::parse_str(&occurrence.id) {
                    selected_in_list |= ui
                        .selectable_value(
                            &mut state.occurrence,
                            Some(OccurrenceId(id)),
                            &occurrence.name,
                        )
                        .clicked();
                }
            }
        });
    let id = state.occurrence;
    if selected_in_list {
        crate::viewport_selection::clear_occurrence(app);
    }
    let state = &mut app.doc.assembly_controls;
    let Some(id) = id else {
        return;
    };
    let Some(occurrence) = projection
        .occurrences
        .iter()
        .find(|o| o.id == id.to_string())
    else {
        return;
    };
    if state.pose_for != Some(id) {
        let t = occurrence.local.translation;
        state.position = [t.x, t.y, t.z];
        state.quaternion = occurrence.local.rotation.components();
        state.pose_for = Some(id);
    }
    let mut grounded = occurrence.grounded;
    if ui.checkbox(&mut grounded, "Grounded").changed() {
        submit(
            app,
            Action::Edit(AssemblyEdit::SetGrounded {
                occurrence: id,
                grounded,
            }),
        );
        return;
    }
    let mut visible = occurrence.visible;
    if ui.checkbox(&mut visible, "Visible").changed() {
        submit(
            app,
            Action::Edit(AssemblyEdit::SetVisibility {
                occurrence: id,
                visible,
            }),
        );
        return;
    }
    if ui.button("Make independent copy").clicked() {
        let definition = ComponentDefId::new();
        app.doc.assembly_controls.definition = Some(definition);
        submit(
            app,
            Action::Edit(AssemblyEdit::MakeIndependent {
                occurrence: id,
                new_definition: definition,
                name: format!("{} copy", occurrence.name),
            }),
        );
        return;
    }
    let state = &mut app.doc.assembly_controls;
    vector(ui, "Position (mm)", &mut state.position);
    ui.horizontal(|ui| {
        ui.label("Rotation quaternion w,x,y,z");
        for value in &mut state.quaternion {
            ui.add(egui::DragValue::new(value).speed(0.01));
        }
    });
    if ui.button("Apply pose").clicked() {
        if projection.joints.iter().any(|joint| {
            !joint.suppressed && (joint.a.occurrence == id || joint.b.occurrence == id)
        }) {
            app.doc.status =
                "Edit and solve the joint before moving a constrained occurrence".into();
            return;
        }
        let q = state.quaternion;
        let p = state.position;
        let Some(rotation) = Quat::try_new(q[0], q[1], q[2], q[3]) else {
            app.doc.status = "Rotation must be a unit quaternion".into();
            return;
        };
        let translation = Vec3d::new(p[0], p[1], p[2]);
        if !translation.is_finite() {
            app.doc.status = "Position must be finite".into();
            return;
        }
        submit(
            app,
            Action::Edit(AssemblyEdit::SetPose {
                occurrence: id,
                pose: RigidTransform::new(rotation, translation),
            }),
        );
    }
}

fn joints(app: &mut App, ui: &mut egui::Ui, projection: &AssemblyProjectionDto) {
    ui.separator();
    ui.label("Joints · local datum frames");
    ui.label("Numeric frames are literal. Picked faces retain geometry identity; X is explicitly entered.");
    let selected = app.doc.topology.occurrence.clone();
    let scene = app
        .doc
        .display
        .as_ref()
        .map(|display| display.scene.identity());
    let state = &mut app.doc.assembly_controls;
    egui::ComboBox::from_id_salt("assembly-joint")
        .selected_text(
            state
                .joint
                .and_then(|id| {
                    projection
                        .joints
                        .iter()
                        .find(|j| j.id == id)
                        .map(|j| j.name.as_str())
                })
                .unwrap_or("New joint"),
        )
        .show_ui(ui, |ui| {
            if ui
                .selectable_value(&mut state.joint, None, "New joint")
                .clicked()
            {
                state.draft = None;
                state.origin_picks.clear();
                state.unbound_sides.clear();
            }
            for joint in &projection.joints {
                if ui
                    .selectable_value(&mut state.joint, Some(joint.id), &joint.name)
                    .clicked()
                {
                    state.draft = Some(joint.clone());
                    state.origin_picks.clear();
                    state.unbound_sides.clear();
                }
            }
        });
    if state.draft.is_none()
        && projection.occurrences.len() >= 2
        && let (Ok(a), Ok(b)) = (
            uuid::Uuid::parse_str(&projection.occurrences[0].id),
            uuid::Uuid::parse_str(&projection.occurrences[1].id),
        )
    {
        let frame = Frame {
            origin: Vec3d::ZERO,
            z: Vec3d::Z,
            x: Vec3d::X,
        };
        state.draft = Some(Joint {
            id: JointId::new(),
            name: "Joint".into(),
            kind: JointKind::Rigid,
            a: JointOrigin {
                occurrence: OccurrenceId(a),
                frame,
            },
            b: JointOrigin {
                occurrence: OccurrenceId(b),
                frame,
            },
            limits: None,
            value: 0.0,
            suppressed: false,
        });
    }
    let Some(draft) = state.draft.as_mut() else {
        ui.label("Add at least two occurrences.");
        return;
    };
    ui.horizontal(|ui| {
        ui.label("Name");
        ui.text_edit_singleline(&mut draft.name);
    });
    egui::ComboBox::from_id_salt("joint-kind")
        .selected_text(format!("{:?}", draft.kind))
        .show_ui(ui, |ui| {
            ui.selectable_value(&mut draft.kind, JointKind::Rigid, "Rigid");
            ui.selectable_value(&mut draft.kind, JointKind::Revolute, "Revolute");
            ui.selectable_value(&mut draft.kind, JointKind::Slider, "Slider");
        });
    origin(ui, "A", &mut draft.a, projection);
    origin_binding_buttons(
        ui,
        (&selected, scene, projection),
        draft.id,
        &draft.a,
        onecad_core::document::component::JointSide::A,
        &mut state.origin_picks,
        &mut state.unbound_sides,
    );
    origin(ui, "B", &mut draft.b, projection);
    origin_binding_buttons(
        ui,
        (&selected, scene, projection),
        draft.id,
        &draft.b,
        onecad_core::document::component::JointSide::B,
        &mut state.origin_picks,
        &mut state.unbound_sides,
    );
    ui.checkbox(&mut draft.suppressed, "Suppressed");
    if draft.kind != JointKind::Rigid {
        let mut limited = draft.limits.is_some();
        if ui.checkbox(&mut limited, "Limits").changed() {
            draft.limits = limited.then_some(JointLimits { min: 0.0, max: 1.0 });
        }
        if let Some(limits) = &mut draft.limits {
            ui.horizontal(|ui| {
                ui.label("Min / max");
                ui.add(egui::DragValue::new(&mut limits.min));
                ui.add(egui::DragValue::new(&mut limits.max));
            });
        }
        ui.horizontal(|ui| {
            ui.label("Rest / drive value (rad or mm)");
            ui.add(egui::DragValue::new(&mut draft.value));
        });
        ui.checkbox(&mut state.drive, "Drive to value during solve");
    } else {
        draft.limits = None;
        state.drive = false;
    }
    let mut action = None;
    if ui.button("Save joint and solve").clicked() {
        let mut joints = projection.joints.clone();
        if let Some(existing) = joints.iter_mut().find(|joint| joint.id == draft.id) {
            *existing = draft.clone();
        } else {
            joints.push(draft.clone());
        }
        state.joint = Some(draft.id);
        let drives = if state.drive {
            vec![(draft.id, draft.value)]
        } else {
            Vec::new()
        };
        action = Some(Action::SolveBound {
            joints,
            drives,
            joint: draft.id,
            picks: state.origin_picks.clone(),
            unbound: state.unbound_sides.clone(),
        });
    }
    if state.joint.is_some() {
        ui.label("Ground each detached component before deleting its joint.");
    }
    if state.joint.is_some() && ui.button("Delete joint and solve").clicked() {
        let id = draft.id;
        let joints = projection
            .joints
            .iter()
            .filter(|j| j.id != id)
            .cloned()
            .collect();
        state.joint = None;
        state.draft = None;
        action = Some(Action::SolveBound {
            joints,
            drives: Vec::new(),
            joint: id,
            picks: Vec::new(),
            unbound: Vec::new(),
        });
    }
    if let Some(report) = &state.report {
        ui.label(format!(
            "Last solve — remaining DOF: {} · residual {:.3e} · redundant {}",
            report.remaining_dof, report.max_residual, report.redundant_constraints
        ));
    }
    if let Some(action) = action {
        submit(app, action);
    }
}

fn origin_binding_buttons(
    ui: &mut egui::Ui,
    context: (
        &Option<crate::viewport_selection::OccurrenceSelection>,
        Option<onecad_scene::SceneIdentity>,
        &AssemblyProjectionDto,
    ),
    joint: JointId,
    origin: &JointOrigin,
    side: onecad_core::document::component::JointSide,
    picks: &mut Vec<super::OriginPick>,
    unbound: &mut Vec<onecad_core::document::component::JointSide>,
) {
    let (selected, scene, projection) = context;
    let candidate = selected.as_ref().filter(|selection| {
        Some(selection.scene) == scene
            && selection.kind == onecad_render::HighlightKind::Face
            && selection.topology_id.starts_with("el_")
            && selection
                .path
                .0
                .last()
                .is_some_and(|key| key.0 == origin.occurrence.0.as_u128())
    });
    let generation = candidate
        .and_then(|_| {
            projection
                .occurrences
                .iter()
                .find(|occurrence| occurrence.id == origin.occurrence.to_string())
        })
        .and_then(|occurrence| {
            projection
                .definitions
                .iter()
                .find(|definition| definition.id == occurrence.definition)
        })
        .and_then(|definition| definition.mesh_generation);
    ui.horizontal(|ui| {
        if ui
            .add_enabled(
                candidate.is_some() && generation.is_some(),
                egui::Button::new("Bind selected face"),
            )
            .clicked()
            && let (Some(selection), Some(mesh_generation)) = (candidate, generation)
        {
            picks.retain(|pick| pick.side != side);
            picks.push(super::OriginPick {
                side,
                selection: selection.clone(),
                mesh_generation,
            });
            unbound.retain(|item| *item != side);
        }
        if ui.button("Use literal datum").clicked() {
            picks.retain(|pick| pick.side != side);
            if !unbound.contains(&side) {
                unbound.push(side);
            }
        }
        if picks.iter().any(|pick| pick.side == side) {
            ui.label("Face selected; save to bind");
        } else if unbound.contains(&side) {
            ui.label("Literal datum selected; save to detach");
        } else if let Some(binding) = projection
            .joint_bindings
            .iter()
            .find(|binding| binding.joint == joint && binding.side == side)
        {
            ui.label(if binding.needs_repair {
                "Face binding needs repair"
            } else if !binding.verified {
                "Face binding awaiting verification"
            } else {
                "Face binding; revalidated when solving"
            });
        } else {
            ui.label("Literal datum");
        }
    });
}

fn origin(
    ui: &mut egui::Ui,
    label: &str,
    origin: &mut JointOrigin,
    projection: &AssemblyProjectionDto,
) {
    egui::ComboBox::from_id_salt(("joint-origin", label))
        .selected_text(
            projection
                .occurrences
                .iter()
                .find(|o| o.id == origin.occurrence.to_string())
                .map_or("Select occurrence", |o| o.name.as_str()),
        )
        .show_ui(ui, |ui| {
            for occurrence in &projection.occurrences {
                if let Ok(id) = uuid::Uuid::parse_str(&occurrence.id) {
                    ui.selectable_value(&mut origin.occurrence, OccurrenceId(id), &occurrence.name);
                }
            }
        });
    ui.label(format!("{label} local datum"));
    frame_vector(ui, "Origin (mm)", &mut origin.frame.origin);
    frame_vector(ui, "Z axis", &mut origin.frame.z);
    frame_vector(ui, "X axis", &mut origin.frame.x);
}
fn frame_vector(ui: &mut egui::Ui, label: &str, v: &mut Vec3d) {
    let mut values = [v.x, v.y, v.z];
    vector(ui, label, &mut values);
    *v = Vec3d::new(values[0], values[1], values[2]);
}
pub(super) fn vector(ui: &mut egui::Ui, label: &str, values: &mut [f64; 3]) {
    ui.horizontal(|ui| {
        ui.label(label);
        for value in values {
            ui.add(egui::DragValue::new(value).speed(0.1));
        }
    });
}
