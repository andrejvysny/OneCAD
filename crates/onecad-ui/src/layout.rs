//! Pure geometry of the floating editor chrome (docs/design/native-ui/final/README.md).
use crate::ui_state::UiState;
use egui::{Pos2, Rect, pos2, vec2};

const TITLEBAR: f32 = 44.0;
const STATUSBAR: f32 = 28.0;
const EDGE: f32 = 10.0;
const TOGGLE: f32 = 44.0;
const EXPLORER_W: f32 = 248.0;
const INSPECTOR_W: f32 = 288.0;
const CARD_W: f32 = 280.0;
const CARD_H: f32 = 320.0;
const WORKSPACE: egui::Vec2 = vec2(196.0, 44.0);

#[derive(Clone, Copy, Debug)]
pub struct Chrome {
    // The bars are egui panels; their rects document the geometry and feed the tests.
    #[allow(dead_code)]
    pub titlebar: Rect,
    #[allow(dead_code)]
    pub statusbar: Rect,
    pub viewport: Rect,
    pub explorer_toggle: Rect,
    pub workspace_button: Rect,
    pub explorer: Rect,
    /// Only present while the Inspector is closed.
    pub inspector_toggle: Option<Rect>,
    pub inspector: Rect,
    pub pill_centre_x: f32,
    /// Running-feature card: left of the Inspector (or the viewport edge), under the pill row.
    pub feature_card: Rect,
    /// Top-right corner of the navigation cube region.
    pub view_cluster: Pos2,
    /// Bottom-left corner of the nav pill.
    pub nav_pill: Pos2,
}

fn feature_card(viewport: Rect, inspector_open: bool) -> Rect {
    let right = viewport.right() - EDGE;
    let edge = if inspector_open {
        right - INSPECTOR_W
    } else {
        right
    };
    Rect::from_min_size(
        pos2(edge - 8.0 - CARD_W, viewport.top() + 62.0),
        vec2(CARD_W, CARD_H),
    )
}

fn pill_centre_x(viewport: Rect, state: &UiState) -> f32 {
    viewport.center().x
        + if state.inspector_open {
            -64.0
        } else if state.explorer_open {
            86.0
        } else {
            0.0
        }
}

pub fn chrome(screen: Rect, state: &UiState) -> Chrome {
    let titlebar = Rect::from_min_size(screen.min, vec2(screen.width(), TITLEBAR));
    let statusbar =
        Rect::from_min_max(pos2(screen.left(), screen.bottom() - STATUSBAR), screen.max);
    let viewport = Rect::from_min_max(titlebar.left_bottom(), statusbar.right_top());
    let (l, t, r, b) = (
        viewport.left(),
        viewport.top(),
        viewport.right(),
        viewport.bottom(),
    );
    let open = state.inspector_open;
    Chrome {
        titlebar,
        statusbar,
        viewport,
        explorer_toggle: Rect::from_min_size(pos2(l + EDGE, t + EDGE), vec2(TOGGLE, TOGGLE)),
        workspace_button: Rect::from_min_size(pos2(l + 62.0, t + EDGE), WORKSPACE),
        explorer: Rect::from_min_max(
            pos2(l + EDGE, t + 62.0),
            pos2(l + EDGE + EXPLORER_W, b - EDGE),
        ),
        inspector_toggle: (!open)
            .then(|| Rect::from_min_size(pos2(r - EDGE - TOGGLE, t + EDGE), vec2(TOGGLE, TOGGLE))),
        inspector: Rect::from_min_max(
            pos2(r - EDGE - INSPECTOR_W, t + EDGE),
            pos2(r - EDGE, b - EDGE),
        ),
        pill_centre_x: pill_centre_x(viewport, state),
        feature_card: feature_card(viewport, open),
        view_cluster: if open {
            pos2(r - 308.0, t + EDGE)
        } else {
            pos2(r - 14.0, t + 62.0)
        },
        nav_pill: pos2(l + if state.explorer_open { 268.0 } else { EDGE }, b - EDGE),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen() -> Rect {
        Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 800.0))
    }

    #[test]
    fn bars_and_viewport() {
        let c = chrome(screen(), &UiState::default());
        assert_eq!(c.titlebar.height(), 44.0);
        assert_eq!(c.statusbar.height(), 28.0);
        assert_eq!(c.viewport.top(), 44.0);
        assert_eq!(c.viewport.bottom(), 772.0);
        assert_eq!(c.explorer.min, pos2(10.0, 106.0));
        assert_eq!(c.explorer.width(), 248.0);
        assert_eq!(c.inspector.width(), 288.0);
        assert_eq!(c.inspector.right(), 1270.0);
    }

    #[test]
    fn pill_offset_follows_panels() {
        let mut s = UiState::default();
        let centre = 640.0;
        assert_eq!(chrome(screen(), &s).pill_centre_x, centre - 64.0);
        s.inspector_open = false;
        assert_eq!(chrome(screen(), &s).pill_centre_x, centre + 86.0);
        s.explorer_open = false;
        assert_eq!(chrome(screen(), &s).pill_centre_x, centre);
    }

    #[test]
    fn cluster_anchor_follows_inspector() {
        let mut s = UiState::default();
        let c = chrome(screen(), &s);
        assert_eq!(c.view_cluster, pos2(1280.0 - 308.0, 54.0));
        assert!(c.inspector_toggle.is_none());
        s.inspector_open = false;
        let c = chrome(screen(), &s);
        assert_eq!(c.view_cluster, pos2(1266.0, 106.0));
        assert_eq!(c.inspector_toggle.map(|r| r.min), Some(pos2(1226.0, 54.0)));
    }

    #[test]
    fn feature_card_sits_left_of_inspector() {
        let mut s = UiState::default();
        let c = chrome(screen(), &s).feature_card;
        assert_eq!(c.min, pos2(1270.0 - 288.0 - 8.0 - 280.0, 106.0));
        assert_eq!(c.width(), 280.0);
        s.inspector_open = false;
        let c = chrome(screen(), &s).feature_card;
        assert_eq!(c.right(), 1270.0 - 8.0);
        assert_eq!(c.top(), 106.0);
    }

    #[test]
    fn nav_pill_follows_explorer() {
        let mut s = UiState::default();
        assert_eq!(chrome(screen(), &s).nav_pill, pos2(268.0, 762.0));
        s.explorer_open = false;
        assert_eq!(chrome(screen(), &s).nav_pill.x, 10.0);
    }
}
