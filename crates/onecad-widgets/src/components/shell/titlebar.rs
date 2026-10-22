//! TitleBar (c-titlebar): h44 drag region with Home, File, centred document name,
//! undo/redo and a command-search look; `bare` keeps only Home, Undo and Redo.

use egui::{Color32, CornerRadius, FontId, Pos2, Rect, Response, Sense, Stroke, Ui, Vec2, vec2};

use super::tabs::{TabStrip, TabsOut, show_strip};
use super::{IconButton, WorkspaceButton};
use crate::components::actions::common::fitted;
use crate::components::tooltip::Tip;
use crate::fonts;
use crate::glyph;
use crate::metrics::{H_TITLEBAR, RADIUS_SM};
use crate::theme::{Look, look};

const TRAFFIC_INSET: f32 = 72.0;
const SEARCH_W: f32 = 220.0;
const BTN: f32 = 28.0;
/// Upper bound for the "File ▾" button, used only to reserve the left cluster.
const FILE_W: f32 = 64.0;

/// Workspace switcher shown after Home; the host owns its popover.
#[derive(Clone, Copy)]
pub struct WorkspaceSpec<'a> {
    pub name: &'a str,
    pub icon: &'a str,
    pub open: bool,
}

pub struct TitleBar<'a> {
    doc_name: &'a str,
    dirty: bool,
    suffix: Option<&'a str>,
    can_undo: bool,
    can_redo: bool,
    home_blocked: Option<&'a str>,
    search_blocked: Option<&'a str>,
    drag_window: bool,
    leading_inset: f32,
    tab_file: bool,
    workspace: Option<WorkspaceSpec<'a>>,
    bare: bool,
    tabs: Option<TabStrip<'a>>,
}

pub struct TitleBarOutput {
    pub home: bool,
    /// Anchor for the host's workspace popover; `None` when no workspace was given.
    pub workspace: Option<Response>,
    /// Anchor for the host's File popover; `None` in the bare bar.
    pub file: Option<Response>,
    pub undo: bool,
    pub redo: bool,
    /// Command search clicked; the host decides what opens.
    pub search: bool,
    /// Tab clicked (tab mode only).
    pub activate: Option<u64>,
    /// Tab × or middle-click.
    pub close: Option<u64>,
    /// "+" clicked.
    pub add: bool,
    /// Painted rect of every tab, so the host can anchor a popover to one.
    pub tab_rects: Vec<(u64, Rect)>,
    /// Widget id of every tab and of "+", so the host can name them for automation.
    pub tab_ids: Vec<(u64, egui::Id)>,
    pub add_id: Option<egui::Id>,
}

impl TitleBarOutput {
    fn plain(home: bool, workspace: Option<Response>) -> Self {
        Self {
            home,
            workspace,
            file: None,
            undo: false,
            redo: false,
            search: false,
            activate: None,
            close: None,
            add: false,
            tab_rects: Vec::new(),
            tab_ids: Vec::new(),
            add_id: None,
        }
    }
}

impl<'a> TitleBar<'a> {
    pub fn new(doc_name: &'a str) -> Self {
        Self {
            doc_name,
            dirty: false,
            suffix: None,
            can_undo: true,
            can_redo: true,
            home_blocked: None,
            search_blocked: None,
            drag_window: true,
            leading_inset: TRAFFIC_INSET,
            tab_file: false,
            workspace: None,
            bare: false,
            tabs: None,
        }
    }
    pub fn dirty(mut self, dirty: bool) -> Self {
        self.dirty = dirty;
        self
    }
    pub fn suffix(mut self, suffix: Option<&'a str>) -> Self {
        self.suffix = suffix;
        self
    }
    pub fn can_undo(mut self, v: bool) -> Self {
        self.can_undo = v;
        self
    }
    pub fn can_redo(mut self, v: bool) -> Self {
        self.can_redo = v;
        self
    }

    /// Whether the empty bar drags / maximises the OS window (default `true`).
    /// Disable Home, showing `reason` in its tooltip.
    pub fn home_blocked(mut self, reason: Option<&'a str>) -> Self {
        self.home_blocked = reason;
        self
    }

    /// Unavailable search remains discoverable without advertising an unbound shortcut.
    pub fn search_blocked(mut self, reason: Option<&'a str>) -> Self {
        self.search_blocked = reason;
        self
    }

    /// Show the workspace button after Home (`None` hides it).
    pub fn workspace(mut self, spec: Option<WorkspaceSpec<'a>>) -> Self {
        self.workspace = spec;
        self
    }

    /// Only Home, Undo and Redo after the traffic lights: no File, name or search.
    pub fn bare(mut self, v: bool) -> Self {
        self.bare = v;
        self
    }

    /// Document tab strip: Home, Undo, Redo on the left, tabs centred, search on the
    /// right; no document name and no File button unless `tab_file`. `None` keeps the
    /// other modes.
    pub fn tabs(mut self, tabs: Option<TabStrip<'a>>) -> Self {
        self.tabs = tabs;
        self
    }

    pub fn drag_window(mut self, v: bool) -> Self {
        self.drag_window = v;
        self
    }

    /// Leading gap before Home; defaults to the macOS traffic-light width. Hosts
    /// without window controls (iPadOS) pass their own inset.
    pub fn leading_inset(mut self, inset: f32) -> Self {
        self.leading_inset = inset;
        self
    }

    /// Tab mode only: show the "File ▾" button after Undo/Redo for hosts without a
    /// native menu bar; its response is returned in `file`.
    pub fn tab_file(mut self, v: bool) -> Self {
        self.tab_file = v;
        self
    }

    pub fn show(self, ui: &mut Ui) -> TitleBarOutput {
        let l = look(ui.ctx());
        let t = &l.theme;
        let size = vec2(ui.available_width(), H_TITLEBAR);
        let (rect, bg) = ui.allocate_exact_size(size, Sense::click_and_drag());
        if self.drag_window {
            window_drag(ui, &bg);
        }
        let p = ui.painter();
        p.rect_filled(rect, 0.0, t.titlebar);
        let y = rect.bottom() - 0.5;
        p.hline(rect.x_range(), y, Stroke::new(1.0, t.border));
        let ws_w = self
            .workspace
            .map(|w| WorkspaceButton::compact_width(ui.painter(), w.name));
        if !self.bare && self.tabs.is_none() {
            self.paint_centre(ui, rect, &l, ws_w);
        }
        let cy = rect.center().y;
        let mut x = rect.left() + self.leading_inset;
        let home_rect = Rect::from_min_size(Pos2::new(x, cy - BTN / 2.0), Vec2::splat(BTN));
        let home = IconButton::new("home", "Home", BTN, 16.0)
            .on(self.tabs.is_some_and(|s| s.home_active))
            .enabled(self.home_blocked.is_none())
            .gap_bg(t.titlebar)
            .tip(Some(Tip::new("Home").reason(self.home_blocked)));
        let home = ui.put(home_rect, home).clicked();
        x += BTN + 4.0;
        let workspace = self.workspace.zip(ws_w).map(|(spec, w)| {
            let r = Rect::from_min_size(Pos2::new(x, cy - 15.0), vec2(w, 30.0));
            let b = WorkspaceButton::compact(spec.name, spec.icon).open(spec.open);
            x += w + 8.0;
            let line = Rect::from_center_size(Pos2::new(x + 0.5, cy), vec2(1.0, 18.0));
            ui.painter().rect_filled(line, 0.0, t.border_strong);
            x += 1.0 + 8.0;
            ui.put(r, b)
        });
        if self.bare {
            let (undo, redo) = self.history_buttons(ui, Pos2::new(x, cy - BTN / 2.0), &l);
            return TitleBarOutput {
                undo,
                redo,
                ..TitleBarOutput::plain(home, workspace)
            };
        }
        if let Some(strip) = self.tabs {
            return self.show_tabs(ui, rect, &l, strip, (x, home, workspace));
        }
        let file = Some(file_button(ui, Pos2::new(x, cy - BTN / 2.0), &l));
        let (search, undo, redo) = self.right_cluster(ui, rect, &l);
        TitleBarOutput {
            file,
            undo,
            redo,
            search,
            ..TitleBarOutput::plain(home, workspace)
        }
    }

    /// Tab mode: undo/redo follow Home on the left, the strip is centred, search on the right.
    fn show_tabs(
        &self,
        ui: &mut Ui,
        rect: Rect,
        l: &Look,
        strip: TabStrip<'_>,
        (x, home, workspace): (f32, bool, Option<Response>),
    ) -> TitleBarOutput {
        let cy = rect.center().y;
        let (undo, redo) = self.history_buttons(ui, Pos2::new(x, cy - BTN / 2.0), l);
        let mut left = x + 2.0 * (BTN + 4.0) + 4.0;
        let file = self.tab_file.then(|| {
            let file = file_button(ui, Pos2::new(left, cy - BTN / 2.0), l);
            left += file.rect.width() + 8.0;
            file
        });
        let search_rect = Rect::from_min_size(
            Pos2::new(rect.right() - 12.0 - SEARCH_W, cy - BTN / 2.0),
            vec2(SEARCH_W, BTN),
        );
        let right = search_rect.left() - 8.0;
        let search = self.search_box(ui, search_rect, l, "Search");
        let c = rect.center().x;
        let sym_w = 2.0 * (c - left).min(right - c).max(0.0);
        let area = Rect::from_min_max(Pos2::new(left, rect.top()), Pos2::new(right, rect.bottom()));
        let TabsOut {
            activate,
            close,
            add,
            rects,
            ids,
            add_id,
        } = show_strip(ui, area, c, sym_w, strip, l);
        TitleBarOutput {
            file,
            undo,
            redo,
            search,
            activate,
            close,
            add,
            tab_rects: rects,
            tab_ids: ids,
            add_id,
            ..TitleBarOutput::plain(home, workspace)
        }
    }

    /// Command-search look plus click; honours `search_blocked`.
    fn search_box(&self, ui: &mut Ui, rect: Rect, l: &Look, hint: &str) -> bool {
        let r = ui
            .add_enabled_ui(self.search_blocked.is_none(), |ui| {
                search_look(ui, rect, l, self.search_blocked.is_none(), hint)
            })
            .inner;
        if let Some(reason) = self.search_blocked {
            r.on_disabled_hover_text(reason).clicked()
        } else {
            r.clicked()
        }
    }

    /// Room for the centred name: symmetric about the bar centre, between the clusters.
    fn centre_width(rect: Rect, ws_w: Option<f32>, inset: f32) -> f32 {
        let ws = ws_w.map_or(0.0, |w| w + 8.0 + 1.0 + 8.0);
        let left = rect.left() + inset + BTN + 4.0 + ws + FILE_W + 8.0;
        let right = rect.right() - 12.0 - SEARCH_W - 8.0 - 2.0 * (BTN + 4.0) - 4.0;
        let c = rect.center().x;
        2.0 * (c - left).min(right - c).max(0.0)
    }

    fn paint_centre(&self, ui: &Ui, rect: Rect, l: &Look, ws_w: Option<f32>) {
        let t = &l.theme;
        let p = ui.painter();
        let avail = Self::centre_width(rect, ws_w, self.leading_inset);
        let suffix = self.suffix.map(|s| {
            let font = FontId::proportional(12.0);
            fitted(p, &format!("— {s}"), font, t.ink5, avail * 0.4)
        });
        let dot_w = if self.dirty { 7.0 + 6.0 } else { 0.0 };
        let suffix_w = suffix.as_ref().map_or(0.0, |g| g.size().x + 6.0);
        let font = FontId::new(13.0, fonts::semibold());
        let name_max = (avail - dot_w - suffix_w - 6.0).max(0.0);
        let name = fitted(p, self.doc_name, font, t.titlebar_text, name_max);
        let total = name.size().x + 6.0 + dot_w + suffix_w;
        let mut x = rect.center().x - total / 2.0;
        let cy = rect.center().y;
        let w = name.size().x;
        p.galley(
            Pos2::new(x, cy - name.size().y / 2.0),
            name,
            t.titlebar_text,
        );
        x += w + 6.0;
        if self.dirty {
            p.circle_filled(Pos2::new(x + 3.5, cy), 3.5, t.ink5);
            x += 7.0 + 6.0;
        }
        if let Some(g) = suffix {
            p.galley(Pos2::new(x, cy - g.size().y / 2.0), g, t.ink5);
        }
    }

    /// Undo then Redo, left to right from `pos`; returns (undo, redo) clicks.
    fn history_buttons(&self, ui: &mut Ui, pos: Pos2, l: &Look) -> (bool, bool) {
        let specs = [
            ("undo", "Undo", self.can_undo, "⌘Z"),
            ("redo", "Redo", self.can_redo, "⌘⇧Z"),
        ];
        let mut hit = [false; 2];
        for (i, (icon, label, enabled, key)) in specs.into_iter().enumerate() {
            let r = Rect::from_min_size(pos + vec2(i as f32 * (BTN + 4.0), 0.0), Vec2::splat(BTN));
            let b = IconButton::new(icon, label, BTN, 16.0)
                .enabled(enabled)
                .gap_bg(l.theme.titlebar)
                .tip(Some(Tip::new(label).shortcut(Some(key))));
            hit[i] = ui.put(r, b).clicked();
        }
        (hit[0], hit[1])
    }

    /// Returns (search, undo, redo) clicks.
    fn right_cluster(&self, ui: &mut Ui, rect: Rect, l: &Look) -> (bool, bool, bool) {
        let t = &l.theme;
        let cy = rect.center().y;
        let mut right = rect.right() - 12.0;
        let search = Rect::from_min_size(
            Pos2::new(right - SEARCH_W, cy - BTN / 2.0),
            vec2(SEARCH_W, BTN),
        );
        let search = self.search_box(ui, search, l, "Search commands");
        right -= SEARCH_W + 8.0;
        let specs = [
            ("redo", "Redo", self.can_redo, Some("⌘⇧Z")),
            ("undo", "Undo", self.can_undo, Some("⌘Z")),
        ];
        let mut hit = [false; 2];
        for (i, (icon, label, enabled, key)) in specs.into_iter().enumerate() {
            let r = Rect::from_min_size(Pos2::new(right - BTN, cy - BTN / 2.0), Vec2::splat(BTN));
            let b = IconButton::new(icon, label, BTN, 16.0)
                .enabled(enabled)
                .gap_bg(t.titlebar)
                .tip(Some(Tip::new(label).shortcut(key)));
            hit[i] = ui.put(r, b).clicked();
            right -= BTN + 4.0;
        }
        (search, hit[1], hit[0])
    }
}

/// Empty bar area drags the window; double-click toggles maximise.
fn window_drag(ui: &Ui, bg: &Response) {
    if bg.drag_started() {
        ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
    }
    if bg.double_clicked() {
        let maxed = ui.input(|i| i.viewport().maximized.unwrap_or(false));
        ui.ctx()
            .send_viewport_cmd(egui::ViewportCommand::Maximized(!maxed));
    }
}

/// "File ▾" text button: h28, title-bar ink, hover fill.
fn file_button(ui: &mut Ui, pos: Pos2, l: &Look) -> Response {
    let t = &l.theme;
    let galley = ui.painter().layout_no_wrap(
        "File".to_owned(),
        FontId::proportional(12.5),
        t.titlebar_text,
    );
    let w = 8.0 + galley.size().x + 4.0 + 10.0 + 8.0;
    let rect = Rect::from_min_size(pos, vec2(w, BTN));
    let r = ui.interact(rect, ui.scope_id().with("titlebar-file"), Sense::click());
    r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, "File"));
    let p = ui.painter();
    if r.hovered() || r.has_focus() {
        p.rect_filled(rect, CornerRadius::same(RADIUS_SM), t.hover);
    }
    if r.has_focus() {
        let (fw, c) = crate::metrics::focus_ring(t);
        p.rect_stroke(
            rect.expand(fw * 0.5),
            CornerRadius::same(RADIUS_SM + 2),
            Stroke::new(fw, c),
            egui::StrokeKind::Outside,
        );
    }
    let cy = rect.center().y;
    let gx = rect.left() + 8.0;
    let gw = galley.size().x;
    p.galley(
        Pos2::new(gx, cy - galley.size().y / 2.0),
        galley,
        t.titlebar_text,
    );
    let chevron = Rect::from_center_size(Pos2::new(gx + gw + 4.0 + 5.0, cy), Vec2::splat(10.0));
    glyph::chevron_down(p, chevron.center(), 12.0, t.ink4);
    r
}

/// 220×28 command-search look: chip fill, search icon, hint text, "⌘K" chip.
fn search_look(ui: &mut Ui, rect: Rect, l: &Look, shortcut: bool, hint: &str) -> Response {
    let t = &l.theme;
    let id = ui.scope_id().with("titlebar-search");
    let r = ui.interact(rect, id, Sense::click());
    r.widget_info(|| {
        egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), "Search commands")
    });
    let p = ui.painter();
    let fill = if r.hovered() { t.hover } else { t.chip };
    p.rect(
        rect,
        CornerRadius::same(RADIUS_SM),
        fill,
        Stroke::new(1.0, t.border_strong),
        egui::StrokeKind::Inside,
    );
    if r.has_focus() {
        let (fw, c) = crate::metrics::focus_ring(t);
        p.rect_stroke(
            rect.expand(fw * 0.5),
            CornerRadius::same(RADIUS_SM + 2),
            Stroke::new(fw, c),
            egui::StrokeKind::Outside,
        );
    }
    let cy = rect.center().y;
    let glyph_box =
        Rect::from_center_size(Pos2::new(rect.left() + 8.0 + 7.0, cy), Vec2::splat(14.0));
    glyph::search(p, glyph_box.center(), 13.0, t.ink5);
    let hint = p.layout_no_wrap(hint.to_owned(), FontId::proportional(12.0), t.ink5);
    let hh = hint.size().y;
    p.galley(Pos2::new(rect.left() + 30.0, cy - hh / 2.0), hint, t.ink5);
    if !shortcut {
        return r;
    }
    let key = p.layout_no_wrap("⌘K".to_owned(), FontId::new(11.0, fonts::mono()), t.ink5);
    let kr = Rect::from_center_size(
        Pos2::new(rect.right() - 8.0 - (key.size().x + 10.0) / 2.0, cy),
        vec2(key.size().x + 10.0, 18.0),
    );
    p.rect_filled(kr, CornerRadius::same(4), t.chip2);
    p.galley(kr.center() - key.size() / 2.0, key, Color32::PLACEHOLDER);
    r
}
