//! Input wiring: raw winit events (device identity, timing, IME) and the
//! viewport's pointer, scroll and pinch callbacks.

use crate::metrics::uptime_ns;
use crate::state::{DragMode, ScrollDevice, State};
use crate::{AppWindow, LOG_INPUT, Shared, env_flag, log_input, place_dim};
use slint::ComponentHandle;
use slint::winit_030::{EventResult, WinitWindowAccessor, winit};
use std::time::Instant;

/// Applies a drag (orbit or pan) to the cursor at viewport-logical (`x`, `y`).
fn drag_to(app: &AppWindow, st: &mut State, x: f32, y: f32) {
    let Some((mode, lx, ly)) = st.drag else {
        return;
    };
    let (dx, dy) = (x - lx, y - ly);
    if dx == 0.0 && dy == 0.0 {
        return;
    }
    match mode {
        DragMode::Orbit => st.cam.orbit(dx, dy),
        DragMode::Pan => st.cam.pan(dx, dy, app.get_vp_height()),
    }
    st.drag = Some((mode, x, y));
    st.camera_changed();
    place_dim(app, st);
    app.window().request_redraw();
}

pub(crate) fn install_raw_input(app: &AppWindow, state: &Shared) {
    let state = state.clone();
    let log_input = env_flag("SPIKE_LOG_INPUT");
    LOG_INPUT.store(log_input, std::sync::atomic::Ordering::Relaxed);
    // Slint buffers CursorMoved and dispatches it from `about_to_wait`; a redraw requested there
    // lands one display refresh later. Driving the drag from the raw event avoids that frame.
    // SPIKE_SLINT_DRAG=1 keeps Slint's path (TouchArea `moved`) for comparison.
    let raw_drag = !env_flag("SPIKE_SLINT_DRAG");
    let weak = app.as_weak();
    app.window().on_winit_window_event(move |w, ev| {
        use winit::event::{MouseScrollDelta, WindowEvent};
        let Ok(mut st) = state.try_borrow_mut() else {
            return EventResult::Propagate;
        };
        let now = Instant::now();
        match ev {
            WindowEvent::MouseWheel { delta, phase, .. } => {
                st.last_raw_input = Some(now);
                st.scroll_device = match delta {
                    MouseScrollDelta::LineDelta(..) => ScrollDevice::Wheel,
                    MouseScrollDelta::PixelDelta(..) => ScrollDevice::Trackpad,
                };
                if log_input {
                    eprintln!(
                        "[spike] raw wheel {delta:?} {phase:?} uptime_ns={}",
                        uptime_ns()
                    );
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                st.last_raw_input = Some(now);
                if log_input {
                    eprintln!("[spike] raw cursor uptime_ns={}", uptime_ns());
                }
                if raw_drag && let Some(app) = weak.upgrade() {
                    let p = position.to_logical::<f32>(f64::from(w.scale_factor()));
                    drag_to(&app, &mut st, p.x - app.get_vp_x(), p.y - app.get_vp_y());
                }
            }
            WindowEvent::MouseInput { .. }
            | WindowEvent::PinchGesture { .. }
            | WindowEvent::RotationGesture { .. }
            | WindowEvent::PanGesture { .. } => st.last_raw_input = Some(now),
            WindowEvent::Ime(ime) => eprintln!("[spike] raw ime {ime:?}"),
            WindowEvent::KeyboardInput { event, .. } if log_input => eprintln!(
                "[spike] raw key {:?} {:?} text={:?} uptime_ns={}",
                event.state,
                event.logical_key,
                event.text,
                uptime_ns()
            ),
            WindowEvent::ModifiersChanged(m) if log_input => {
                eprintln!("[spike] raw modifiers {:?}", m.state())
            }
            _ => {}
        }
        EventResult::Propagate
    });
}

pub(crate) fn install_viewport_input(app: &AppWindow, state: &Shared) {
    let s = state.clone();
    app.on_vp_down(move |button, x, y, shift| {
        log_input(format_args!(
            "vp-down button={button} ({x:.1},{y:.1}) shift={shift}"
        ));
        let mode = if button == 0 && !shift {
            DragMode::Orbit
        } else {
            DragMode::Pan
        };
        s.borrow_mut().drag = Some((mode, x, y));
    });
    let (s, weak) = (state.clone(), app.as_weak());
    app.on_vp_move(move |x, y| {
        let Some(app) = weak.upgrade() else { return };
        drag_to(&app, &mut s.borrow_mut(), x, y);
    });
    let s = state.clone();
    app.on_vp_up(move || {
        log_input(format_args!("vp-up"));
        s.borrow_mut().drag = None;
    });
    let (s, weak) = (state.clone(), app.as_weak());
    app.on_vp_hover(move |x, y| {
        let Some(app) = weak.upgrade() else { return };
        let st = s.borrow();
        if let Some(p) = st.ground_point(x, y, app.get_vp_width(), app.get_vp_height()) {
            app.set_cursor(format!("X {:>8.2}  Y {:>8.2}  Z {:>6.2} mm", p.x, p.y, p.z).into());
        }
    });
    let (s, weak) = (state.clone(), app.as_weak());
    app.on_vp_scroll(move |dx, dy, x, y, shift, ctrl| {
        let Some(app) = weak.upgrade() else {
            return false;
        };
        let mut st = s.borrow_mut();
        log_input(format_args!(
            "vp-scroll d=({dx:.1},{dy:.1}) device={:?} shift={shift} ctrl={ctrl}",
            st.scroll_device
        ));
        let (w, h) = (app.get_vp_width(), app.get_vp_height());
        match (st.scroll_device, shift, ctrl) {
            (ScrollDevice::Wheel, _, _) | (_, _, true) => {
                st.cam.zoom_at(0.9985f32.powf(dy), x, y, w, h)
            }
            (ScrollDevice::Trackpad, true, _) => st.cam.orbit(-dx, -dy),
            (ScrollDevice::Trackpad, false, _) => st.cam.pan(dx, dy, h),
        }
        st.camera_changed();
        place_dim(&app, &st);
        app.window().request_redraw();
        true
    });
    let (s, weak) = (state.clone(), app.as_weak());
    app.on_vp_gesture(move |phase, scale, rot, cx, cy| {
        log_input(format_args!(
            "vp-gesture phase={phase} scale={scale:.3} rot={rot:.1} c=({cx:.1},{cy:.1})"
        ));
        let Some(app) = weak.upgrade() else { return };
        let mut st = s.borrow_mut();
        st.drag = None;
        match (phase, st.gesture) {
            (0, _) => st.gesture = Some((scale, cx, cy)),
            (1, Some((s0, x0, y0))) => {
                let (w, h) = (app.get_vp_width(), app.get_vp_height());
                st.cam.zoom_at(s0 / scale.max(1e-3), cx, cy, w, h);
                st.cam.pan(cx - x0, cy - y0, h);
                st.gesture = Some((scale, cx, cy));
                st.camera_changed();
                place_dim(&app, &st);
                app.window().request_redraw();
            }
            _ => st.gesture = None,
        }
    });
}
