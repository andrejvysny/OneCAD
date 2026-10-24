//! Pure geometry of the floating editor chrome (docs/design/native-ui/final/README.md).
use crate::ui_state::UiState;
use egui::{Pos2, Rect, Vec2, pos2, vec2};

const TITLEBAR: f32 = 44.0;
const EDGE: f32 = 10.0;
const TOGGLE: f32 = 32.0;
/// Collapsed explorer chip: toggle | divider | workspace selector, in a 3px padded card.
const EXPLORER_CHIP: Vec2 = vec2(97.0, 40.0);
const EXPLORER_W: f32 = 248.0;
const INSPECTOR_W: f32 = 288.0;
const CARD_W: f32 = 280.0;
const CARD_H: f32 = 320.0;
// Header, footer and dividers leave at least 80px of scrollable parameters.
const CARD_MIN_H: f32 = 180.0;
// NavPill has five 32px buttons, four 2px gaps, 3px padding and a 1px border.
// Keep this clearance aligned with onecad-widgets' overlays/nav.rs.
const NAV_PILL_SIZE: Vec2 = vec2(176.0, 40.0);
pub(crate) const CUBE: f32 = 104.0;
const CONTROL_W: f32 = 36.0;
const CONTROLS_H: f32 = 124.0;

#[derive(Clone, Copy, Debug)]
pub struct Chrome {
    // The bars are egui panels; their rects document the geometry and feed the tests.
    #[allow(dead_code)]
    pub titlebar: Rect,
    pub viewport: Rect,
    pub explorer_toggle: Rect,
    pub toolbar_left: f32,
    pub explorer_open: bool,
    pub explorer: Rect,
    /// Only present while the Inspector is closed.
    pub inspector_toggle: Option<Rect>,
    pub inspector_open: bool,
    pub inspector: Rect,
    pub pill_centre_x: f32,
    /// Running-feature card below or beside navigation, clear of visible sidebars.
    pub feature_card: Rect,
    /// Top-right corner of the navigation cube region.
    pub view_cluster: Pos2,
    pub view_controls: Rect,
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

fn collapsed_inspector_toggle(controls: Rect) -> Rect {
    Rect::from_min_size(
        pos2(controls.center().x - TOGGLE / 2.0, controls.bottom() + 8.0),
        vec2(TOGGLE, TOGGLE),
    )
}

pub fn chrome(screen: Rect, state: &UiState) -> Chrome {
    let titlebar = Rect::from_min_size(screen.min, vec2(screen.width(), TITLEBAR));
    let viewport = Rect::from_min_max(titlebar.left_bottom(), screen.max);
    let (l, t, r, b) = (
        viewport.left(),
        viewport.top(),
        viewport.right(),
        viewport.bottom(),
    );
    let open = state.inspector_open;
    let view_cluster = if open {
        pos2(r - 308.0, t + EDGE)
    } else {
        pos2(r - 14.0, t + EDGE)
    };
    let view_controls = Rect::from_min_size(
        pos2(
            view_cluster.x - CUBE / 2.0 - CONTROL_W / 2.0,
            view_cluster.y + CUBE + 4.0,
        ),
        vec2(CONTROL_W, CONTROLS_H),
    );
    Chrome {
        titlebar,
        viewport,
        explorer_toggle: Rect::from_min_size(pos2(l + EDGE, t + EDGE), EXPLORER_CHIP),
        toolbar_left: l
            + EDGE
            + if state.explorer_open {
                EXPLORER_W
            } else {
                EXPLORER_CHIP.x
            }
            + EDGE,
        explorer_open: state.explorer_open,
        explorer: Rect::from_min_max(
            pos2(l + EDGE, t + EDGE),
            pos2(l + EDGE + EXPLORER_W, b - EDGE),
        ),
        inspector_toggle: Some(collapsed_inspector_toggle(view_controls)),
        inspector_open: open,
        inspector: Rect::from_min_max(
            pos2(r - EDGE - INSPECTOR_W, t + EDGE),
            pos2(r - EDGE, b - EDGE),
        ),
        pill_centre_x: viewport.center().x,
        feature_card: feature_card(viewport, open),
        view_cluster,
        view_controls,
        nav_pill: pos2(l + if state.explorer_open { 268.0 } else { EDGE }, b - EDGE),
    }
}

/// Floating sidebars cover the image without reducing its camera viewport.
pub(crate) fn fit_work_area(
    screen: Rect,
    viewport: Rect,
    state: &UiState,
) -> Option<onecad_scene::SafeRect> {
    if !screen.is_finite() || !viewport.is_finite() || !viewport.is_positive() {
        return None;
    }
    let chrome = chrome(screen, state);
    let left = if state.explorer_open {
        chrome.explorer.right()
    } else {
        chrome.viewport.left() + EDGE
    };
    let right = if state.inspector_open {
        chrome.inspector.left()
    } else {
        chrome.viewport.right() - EDGE
    };
    let (left, right) = (left.max(viewport.left()), right.min(viewport.right()));
    (right > left).then_some(onecad_scene::SafeRect {
        x: f64::from(left - viewport.left()),
        y: 0.0,
        width: f64::from(right - left),
        height: f64::from(viewport.height()),
    })
}

impl Chrome {
    /// Move controls beside the cube while parameters occupy the space beneath it.
    pub fn with_feature_card(mut self) -> Self {
        if !self.inspector_open {
            self.view_cluster = pos2(self.viewport.right() - 14.0, self.viewport.top() + EDGE);
        }
        self.view_controls = Rect::from_min_size(
            pos2(
                self.view_cluster.x - CUBE - 8.0 - CONTROL_W,
                self.view_cluster.y + 20.0,
            ),
            vec2(CONTROL_W, CONTROLS_H),
        );
        self.inspector_toggle = self
            .inspector_toggle
            .map(|_| collapsed_inspector_toggle(self.view_controls));
        let controls_bottom = self
            .inspector_toggle
            .map_or(self.view_controls.bottom(), |toggle| toggle.bottom());
        let top = (self.view_cluster.y + CUBE).max(controls_bottom) + 8.0;
        self.feature_card = Rect::from_min_size(
            pos2(self.feature_card.left(), top),
            vec2(
                CARD_W,
                CARD_H.min((self.viewport.bottom() - EDGE - top).max(0.0)),
            ),
        );
        if self.feature_card.height() < CARD_MIN_H
            && let Some(card) = self.compact_feature_card()
        {
            self.feature_card = card;
        }
        self
    }

    fn compact_feature_card(&self) -> Option<Rect> {
        // A keyboard can leave less space below navigation than the card's scroll
        // minimum plus header/footer. Use free width without hiding any chrome.
        let top = self.viewport.top() + 62.0;
        let mut card = Rect::from_min_size(
            pos2(self.view_controls.left() - EDGE - CARD_W, top),
            vec2(CARD_W, CARD_H.min(self.viewport.bottom() - EDGE - top)),
        );
        let nav = Rect::from_min_size(self.nav_pill - vec2(0.0, NAV_PILL_SIZE.y), NAV_PILL_SIZE);
        if card.left() < nav.right() && card.right() > nav.left() {
            card.max.y = card.bottom().min(nav.top() - EDGE);
        }
        if card.height() < CARD_MIN_H || !self.viewport.contains_rect(card) {
            return None;
        }
        let cube = Rect::from_min_size(self.view_cluster - vec2(CUBE, 0.0), vec2(CUBE, CUBE));
        let occupied = [
            if self.explorer_open {
                self.explorer
            } else {
                self.explorer_toggle
            },
            if self.inspector_open {
                self.inspector
            } else {
                Rect::NOTHING
            },
            cube,
            self.view_controls,
            nav,
            Rect::from_min_size(self.viewport.min, vec2(self.viewport.width(), EDGE + 48.0)),
        ];
        occupied
            .into_iter()
            .chain(self.inspector_toggle)
            .all(|region| !card.intersect(region).is_positive())
            .then_some(card)
    }
}

#[cfg(test)]
#[path = "layout_fit_tests.rs"]
mod fit_tests;

#[cfg(test)]
#[path = "layout_keyboard_tests.rs"]
mod keyboard_tests;

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
        assert_eq!(c.viewport.top(), 44.0);
        assert_eq!(c.viewport.bottom(), 800.0);
        assert_eq!(c.explorer.bottom(), 790.0);
        assert_eq!(c.inspector.bottom(), 790.0);
        assert_eq!(c.explorer.min, pos2(10.0, 54.0));
        assert_eq!(c.explorer.width(), 248.0);
        assert_eq!(c.inspector.width(), 288.0);
        assert_eq!(c.inspector.right(), 1270.0);
    }

    #[test]
    fn pill_stays_centred_when_panels_change() {
        let mut s = UiState::default();
        let centre = 640.0;
        assert_eq!(chrome(screen(), &s).pill_centre_x, centre);
        s.inspector_open = false;
        assert_eq!(chrome(screen(), &s).pill_centre_x, centre);
        s.explorer_open = false;
        assert_eq!(chrome(screen(), &s).pill_centre_x, centre);
    }

    #[test]
    fn cluster_anchor_follows_inspector() {
        let mut s = UiState::default();
        let c = chrome(screen(), &s);
        assert_eq!(c.view_cluster, pos2(1280.0 - 308.0, 54.0));
        // The one inspector toggle stays under the view controls whether the panel is open.
        let open_toggle = c.inspector_toggle.unwrap();
        assert_eq!(open_toggle.center().x, c.view_controls.center().x);
        assert!(!open_toggle.intersects(c.inspector));
        s.inspector_open = false;
        let c = chrome(screen(), &s);
        assert_eq!(c.view_cluster, pos2(1266.0, 54.0));
        let toggle = c.inspector_toggle.unwrap();
        assert_eq!(toggle.center().x, c.view_controls.center().x);
        assert_eq!(toggle.top(), c.view_controls.bottom() + 8.0);
        assert_eq!(toggle.size(), vec2(32.0, 32.0));
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
    fn active_card_cube_and_controls_fit_all_panel_states_at_minimum_size() {
        for size in [vec2(900.0, 560.0), vec2(1280.0, 820.0)] {
            for explorer_open in [false, true] {
                for inspector_open in [false, true] {
                    let state = UiState {
                        explorer_open,
                        inspector_open,
                        ..Default::default()
                    };
                    let chrome =
                        chrome(Rect::from_min_size(Pos2::ZERO, size), &state).with_feature_card();
                    let cube = Rect::from_min_size(
                        chrome.view_cluster - vec2(CUBE, 0.0),
                        vec2(CUBE, CUBE),
                    );
                    let regions = [chrome.feature_card, cube, chrome.view_controls];
                    for (index, region) in regions.iter().enumerate() {
                        assert!(chrome.viewport.contains_rect(*region));
                        for other in &regions[index + 1..] {
                            assert!(!region.intersect(*other).is_positive());
                        }
                        if explorer_open {
                            assert!(!region.intersect(chrome.explorer).is_positive());
                        }
                        if inspector_open {
                            assert!(!region.intersect(chrome.inspector).is_positive());
                        }
                        if let Some(toggle) = chrome.inspector_toggle {
                            assert!(!region.intersect(toggle).is_positive());
                        }
                    }
                    assert!(chrome.feature_card.height() >= 296.0);
                }
            }
        }
    }

    #[test]
    fn collapsed_chip_clears_toolbar() {
        let s = UiState {
            explorer_open: false,
            ..Default::default()
        };
        let c = chrome(screen(), &s);
        assert_eq!(c.explorer_toggle.size(), EXPLORER_CHIP);
        assert_eq!(c.toolbar_left, c.explorer_toggle.right() + EDGE);
    }

    #[test]
    fn nav_pill_follows_explorer() {
        let mut s = UiState::default();
        assert_eq!(chrome(screen(), &s).nav_pill, pos2(268.0, 790.0));
        s.explorer_open = false;
        assert_eq!(chrome(screen(), &s).nav_pill.x, 10.0);
    }
}
