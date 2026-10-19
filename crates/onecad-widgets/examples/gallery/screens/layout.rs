//! Pure rect maths for a screen stage (README "02a Editor" / "03a" / "03b").

use egui::{Pos2, Rect, Vec2, pos2, vec2};
use onecad_widgets::metrics::{EDGE_INSET, H_STATUSBAR, H_TITLEBAR};

pub const STAGE: Vec2 = vec2(1280.0, 800.0);
pub const EXPLORER_W: f32 = 248.0;
pub const INSPECTOR_W: f32 = 288.0;
pub const TOGGLE: f32 = 44.0;
pub const WORKSPACE_W: f32 = 196.0;
pub const VIEW_CUBE: f32 = 72.0;

/// Mock pill shift from the viewport centre. README: Inspector open = -64, nothing else
/// moves it; the design mocks also sit +86 right while only the Explorer is open.
const PILL_SHIFT_INSPECTOR: f32 = -64.0;
const PILL_SHIFT_EXPLORER: f32 = 86.0;

#[derive(Clone, Copy, Debug)]
pub struct Flags {
    pub explorer_open: bool,
    pub inspector_open: bool,
}

pub struct Layout {
    pub stage: Rect,
    pub titlebar: Rect,
    pub statusbar: Rect,
    pub viewport: Rect,
    pub explorer_toggle: Rect,
    pub workspace: Rect,
    pub explorer: Rect,
    pub inspector: Rect,
    pub inspector_toggle: Rect,
    pub pill_cx: f32,
    pub pill_top: f32,
    pub strip_top: f32,
    /// Top-left of the view cluster.
    pub view_cluster: Pos2,
    /// Left edge and bottom edge of the nav pill.
    pub nav_pill: Pos2,
}

impl Layout {
    pub fn new(stage: Rect, flags: Flags) -> Self {
        let titlebar = Rect::from_min_size(stage.min, vec2(stage.width(), H_TITLEBAR));
        let statusbar =
            Rect::from_min_max(pos2(stage.left(), stage.bottom() - H_STATUSBAR), stage.max);
        let viewport = Rect::from_min_max(titlebar.left_bottom(), statusbar.right_top());
        let v = viewport;
        let inset = EDGE_INSET;
        let (view_right, view_top) = if flags.inspector_open {
            (inset + INSPECTOR_W + inset, inset)
        } else {
            (14.0, 62.0)
        };
        let nav_left = if flags.explorer_open {
            inset + EXPLORER_W + inset
        } else {
            inset
        };
        Self {
            stage,
            titlebar,
            statusbar,
            viewport,
            explorer_toggle: Rect::from_min_size(v.min + vec2(inset, inset), Vec2::splat(TOGGLE)),
            workspace: Rect::from_min_size(
                v.min + vec2(inset + TOGGLE + 8.0, inset),
                vec2(WORKSPACE_W, TOGGLE),
            ),
            explorer: Rect::from_min_max(
                v.min + vec2(inset, 62.0),
                pos2(v.left() + inset + EXPLORER_W, v.bottom() - inset),
            ),
            inspector: Rect::from_min_max(
                pos2(v.right() - inset - INSPECTOR_W, v.top() + inset),
                v.max - vec2(inset, inset),
            ),
            inspector_toggle: Rect::from_min_size(
                pos2(v.right() - inset - TOGGLE, v.top() + inset),
                Vec2::splat(TOGGLE),
            ),
            pill_cx: v.center().x + pill_shift(flags),
            pill_top: v.top() + inset,
            strip_top: v.top() + 62.0,
            view_cluster: pos2(v.right() - view_right - VIEW_CUBE, v.top() + view_top),
            nav_pill: pos2(v.left() + nav_left, v.bottom() - inset),
        }
    }
}

fn pill_shift(f: Flags) -> f32 {
    if f.inspector_open {
        PILL_SHIFT_INSPECTOR
    } else if f.explorer_open {
        PILL_SHIFT_EXPLORER
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lay(explorer_open: bool, inspector_open: bool) -> Layout {
        let stage = Rect::from_min_size(Pos2::ZERO, STAGE);
        Layout::new(
            stage,
            Flags {
                explorer_open,
                inspector_open,
            },
        )
    }

    #[test]
    fn chrome_bands() {
        let l = lay(true, true);
        assert_eq!(l.titlebar.height(), 44.0);
        assert_eq!(l.statusbar.height(), 28.0);
        assert_eq!(l.viewport.width(), 1280.0);
        assert_eq!(l.viewport.height(), 800.0 - 44.0 - 28.0);
        assert_eq!(l.explorer.width(), 248.0);
        assert_eq!(l.inspector.width(), 288.0);
        assert_eq!(l.explorer.top(), l.viewport.top() + 62.0);
        assert_eq!(l.inspector.top(), l.viewport.top() + 10.0);
        assert_eq!(l.inspector.bottom(), l.viewport.bottom() - 10.0);
    }

    #[test]
    fn pill_shifts_64_left_when_inspector_open() {
        let c = lay(true, true).viewport.center().x;
        assert_eq!(lay(true, true).pill_cx, c - 64.0);
        assert_eq!(lay(false, true).pill_cx, c - 64.0);
        assert_eq!(lay(false, false).pill_cx, c);
    }

    #[test]
    fn nav_pill_left_follows_explorer() {
        assert_eq!(lay(true, false).nav_pill.x, 268.0);
        assert_eq!(lay(false, false).nav_pill.x, 10.0);
        assert_eq!(lay(false, false).nav_pill.y, 800.0 - 28.0 - 10.0);
    }

    #[test]
    fn view_cluster_moves_left_of_inspector() {
        let closed = lay(true, false);
        assert_eq!(closed.view_cluster.x, 1280.0 - 14.0 - 72.0);
        assert_eq!(closed.view_cluster.y, closed.viewport.top() + 62.0);
        let open = lay(true, true);
        assert_eq!(open.view_cluster.x, 1280.0 - 308.0 - 72.0);
        assert_eq!(open.view_cluster.y, open.viewport.top() + 10.0);
    }
}
