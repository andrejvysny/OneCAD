//! Body tree, dimension input and menu commands, including the explicit undo routing
//! (focused text field first, then the document stack).

use crate::state::{State, format_mm};
use crate::{AppWindow, BodyRow, LOG_INPUT, Shared, expr, place_dim, start_bench};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};
use std::rc::Rc;

fn refresh_selection(app: &AppWindow, st: &State, rows: &VecModel<BodyRow>) {
    for (i, b) in st.bodies.iter().enumerate() {
        let sel = st.selected == Some(i);
        if let Some(mut r) = rows.row_data(i)
            && (r.selected != sel || r.visible != b.visible)
        {
            r.selected = sel;
            r.visible = b.visible;
            rows.set_row_data(i, r);
        }
    }
    let Some(b) = st.selected.and_then(|i| st.bodies.get(i)) else {
        app.set_sel_name(SharedString::new());
        return;
    };
    let size = b.max - b.min;
    let props: Vec<(SharedString, SharedString)> = [
        ("Triangles", b.tris.to_string()),
        ("Vertices", b.verts.to_string()),
        ("Faces", b.faces.to_string()),
        ("Edges", b.edges.to_string()),
        ("Size X", format_mm(f64::from(size.x))),
        ("Size Y", format_mm(f64::from(size.y))),
        ("Size Z", format_mm(f64::from(size.z))),
        (
            "Visible",
            if b.visible { "yes".into() } else { "no".into() },
        ),
        ("MESH1", b.file.chars().take(13).collect::<String>() + "…"),
    ]
    .into_iter()
    .map(|(l, v)| (l.into(), v.into()))
    .collect();
    app.set_sel_props(ModelRc::new(VecModel::from(props)));
    app.set_sel_name(b.label.clone().into());
}

pub(crate) fn install_tree(app: &AppWindow, state: &Shared, rows: &Rc<VecModel<BodyRow>>) {
    let (s, weak, r) = (state.clone(), app.as_weak(), rows.clone());
    app.on_body_clicked(move |i| {
        let Some(app) = weak.upgrade() else { return };
        let mut st = s.borrow_mut();
        let i = i as usize;
        st.selected = if st.selected == Some(i) {
            None
        } else {
            Some(i)
        };
        st.bodies_dirty = true;
        st.camera_changed();
        refresh_selection(&app, &st, &r);
        app.window().request_redraw();
    });
    let (s, weak, r) = (state.clone(), app.as_weak(), rows.clone());
    app.on_body_toggle(move |i| {
        let Some(app) = weak.upgrade() else { return };
        let mut st = s.borrow_mut();
        if let Some(b) = st.bodies.get_mut(i as usize) {
            b.visible = !b.visible;
        }
        st.bodies_dirty = true;
        st.camera_changed();
        refresh_selection(&app, &st, &r);
        app.window().request_redraw();
    });
}

fn set_dim_value(app: &AppWindow, st: &State) {
    app.set_dim_text(format!("{:.2}", st.dim_value).into());
    app.set_dim_preview(SharedString::new());
    app.set_dim_error(false);
    let hist: Vec<SharedString> = st
        .dim_undo
        .iter()
        .chain(std::iter::once(&st.dim_value))
        .rev()
        .take(6)
        .map(|v| format_mm(*v).into())
        .collect();
    app.set_dim_history(ModelRc::new(VecModel::from(hist)));
    app.set_doc_dirty(!st.dim_undo.is_empty());
}

pub(crate) fn install_dimension(app: &AppWindow, state: &Shared) {
    let weak = app.as_weak();
    app.on_dim_edited(move |text| {
        let Some(app) = weak.upgrade() else { return };
        match expr::eval_mm(&text) {
            Ok(v) => {
                app.set_dim_preview(format!("= {}", format_mm(v)).into());
                app.set_dim_error(false);
            }
            Err(e) => {
                app.set_dim_preview(e.0.into());
                app.set_dim_error(true);
            }
        }
    });
    let (s, weak) = (state.clone(), app.as_weak());
    app.on_dim_commit(move |text| {
        let Some(app) = weak.upgrade() else { return };
        let mut st = s.borrow_mut();
        match expr::eval_mm(&text) {
            Ok(v) => {
                let old = st.dim_value;
                st.dim_undo.push(old);
                st.dim_redo.clear();
                st.dim_value = v;
                eprintln!("[spike] dim commit '{text}' -> {v} mm");
                app.set_status(format!("Height = {}", format_mm(v)).into());
                set_dim_value(&app, &st);
                app.invoke_dim_end();
            }
            Err(e) => app.set_status(format!("Invalid expression: {}", e.0).into()),
        }
    });
    let (s, weak) = (state.clone(), app.as_weak());
    app.on_dim_cancel(move || {
        let Some(app) = weak.upgrade() else { return };
        eprintln!("[spike] dim cancel");
        set_dim_value(&app, &s.borrow());
        app.set_status("Edit cancelled".into());
        app.invoke_dim_end();
    });
    app.on_dim_key(|text, meta, control| {
        if meta || control || LOG_INPUT.load(std::sync::atomic::Ordering::Relaxed) {
            eprintln!(
                "[spike] text-field key {:?} meta={meta} control={control}",
                text.as_str()
            );
        }
    });
}

fn undo_redo(app: &AppWindow, st: &mut State, undo: bool) {
    if app.get_dim_focused() {
        eprintln!(
            "[spike] menu {} -> text field",
            if undo { "undo" } else { "redo" }
        );
        if undo {
            app.invoke_dim_text_undo()
        } else {
            app.invoke_dim_text_redo()
        }
        return;
    }
    let (from, to) = if undo {
        (&mut st.dim_undo, &mut st.dim_redo)
    } else {
        (&mut st.dim_redo, &mut st.dim_undo)
    };
    if let Some(v) = from.pop() {
        to.push(st.dim_value);
        st.dim_value = v;
    }
    eprintln!(
        "[spike] menu {} -> document (value {})",
        if undo { "undo" } else { "redo" },
        st.dim_value
    );
    app.set_status(
        format!(
            "{} · Height = {}",
            if undo { "Undo" } else { "Redo" },
            format_mm(st.dim_value)
        )
        .into(),
    );
    set_dim_value(app, st);
}

pub(crate) fn install_menu(app: &AppWindow, state: &Shared) {
    let (s, weak) = (state.clone(), app.as_weak());
    app.on_menu(move |id| {
        let Some(app) = weak.upgrade() else { return };
        let mut st = s.borrow_mut();
        match id.as_str() {
            "undo" => undo_redo(&app, &mut st, true),
            "redo" => undo_redo(&app, &mut st, false),
            "edit-dim" => app.invoke_dim_begin(),
            "home" => st.cam = st.home,
            "fit" => st.fit_visible(app.get_vp_width() / app.get_vp_height().max(1.0)),
            "ortho" | "ortho-on" | "ortho-off" => {
                st.cam.ortho = match id.as_str() {
                    "ortho-on" => true,
                    "ortho-off" => false,
                    _ => !st.cam.ortho,
                };
                app.set_ortho(st.cam.ortho);
            }
            "bench" => start_bench(&mut st, 600, false),
            "report" => {
                st.metrics.report("manual");
                st.metrics.reset();
            }
            "save" => app.set_status("Save is a no-op in the spike".into()),
            _ => {}
        }
        eprintln!("[spike] menu {id}");
        st.camera_changed();
        place_dim(&app, &st);
        app.window().request_redraw();
    });
}
