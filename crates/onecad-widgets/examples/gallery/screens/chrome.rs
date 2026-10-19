//! Shared editor chrome for the mock screens: title bar, panels, toggles, overlays,
//! the centred pill and session strip. Everything is drawn inside the stage rect with
//! child `Ui`s (no `egui::Area`), so a screen scrolls with the page.

use egui::{Color32, Pos2, Rect, Ui, UiBuilder, Vec2, pos2, vec2};
use onecad_widgets::Theme;
use onecad_widgets::components::actions::{
    PillAccent, PillMode, SelectionLabel, SelectionVariant, ToolButton, ToolPill, ToolState,
    pill_divider,
};
use onecad_widgets::components::overlays::{
    NavButton, NavPill, SessionStrip, SquareButton, StripAction, StripVariant, ViewCluster,
};
use onecad_widgets::components::shell::{
    PanelToggle, Side, StatusBar, StatusItem, TitleBar, WorkspaceButton,
};
use onecad_widgets::components::tooltip::Tip;
use onecad_widgets::frame;
use onecad_widgets::icon::{self, IconStyle};
use onecad_widgets::metrics::{Elevation, RADIUS_XL};

use super::explorer::{self, ExplorerMode};
use super::inspector::{self, InspectorView};
use super::layout::{Flags, Layout, VIEW_CUBE};

pub struct Cx<'a> {
    pub t: &'a Theme,
    pub dark: bool,
    pub lay: Layout,
}

impl<'a> Cx<'a> {
    pub fn new(t: &'a Theme, dark: bool, stage: Rect, flags: Flags) -> Self {
        Self {
            t,
            dark,
            lay: Layout::new(stage, flags),
        }
    }
}

/// Run `add` in a child `Ui` limited to `rect`.
pub fn at(ui: &mut Ui, rect: Rect, add: impl FnOnce(&mut Ui)) {
    ui.scope_builder(UiBuilder::new().max_rect(rect), add);
}

/// Content-sized widget centred on `cx`: the width measured last frame offsets the
/// start, so the first frame is one repaint off.
pub fn centred(ui: &mut Ui, cx: f32, top: f32, name: &str, add: impl FnOnce(&mut Ui)) {
    let key = ui.scope_id().with(name);
    let known = ui.data(|d| d.get_temp::<f32>(key)).unwrap_or(0.0);
    let rect = Rect::from_min_size(pos2(cx - known / 2.0, top), vec2(1000.0, 80.0));
    let width = ui
        .scope_builder(UiBuilder::new().max_rect(rect), add)
        .response
        .rect
        .width();
    if (width - known).abs() > 0.5 {
        ui.data_mut(|d| d.insert_temp(key, width));
        ui.ctx().request_repaint();
    }
}

/// Floating panel chrome (same frame as `FloatingPanel`, minus the Area).
pub fn panel(ui: &mut Ui, cx: &Cx, rect: Rect, add: impl FnOnce(&mut Ui)) {
    at(ui, rect, |ui| {
        let inner = rect.size() - Vec2::splat(2.0);
        frame::surface(cx.t, cx.dark, RADIUS_XL, Elevation::Panel).show(ui, |ui| {
            ui.set_min_size(inner);
            ui.set_max_size(inner);
            ui.set_clip_rect(ui.clip_rect().intersect(ui.max_rect()));
            add(ui);
        });
    });
}

pub struct Status<'a> {
    pub sketch: bool,
    pub dot: Option<Color32>,
    pub text: &'a str,
    pub hint: Option<&'a str>,
    pub persp: Option<bool>,
    pub items: &'a [StatusItem<'a>],
}

pub struct Base<'a> {
    pub explorer: Option<ExplorerMode>,
    pub inspector: Option<InspectorView>,
    pub sketch: bool,
    /// Accent dot on a hidden Inspector's toggle ("selection waiting").
    pub dot: bool,
    pub isolate: bool,
    pub status: Status<'a>,
}

impl Base<'_> {
    pub fn flags(&self) -> Flags {
        Flags {
            explorer_open: self.explorer.is_some(),
            inspector_open: self.inspector.is_some(),
        }
    }
}

/// Viewport fill plus every always-present piece of chrome.
pub fn base(ui: &mut Ui, cx: &Cx, b: &Base) {
    let canvas = if b.sketch {
        cx.t.canvas_sketch
    } else {
        cx.t.canvas_model
    };
    ui.painter().rect_filled(cx.lay.viewport, 0.0, canvas);
    title_bar(ui, cx);
    status_bar(ui, cx, &b.status);
    toggles(ui, cx, b);
    if let Some(mode) = b.explorer {
        panel(ui, cx, cx.lay.explorer, |ui| explorer::show(ui, cx, mode));
    }
    if let Some(view) = b.inspector {
        panel(ui, cx, cx.lay.inspector, |ui| inspector::show(ui, cx, view));
    }
    view_cluster(ui, cx, b.sketch);
    nav_pill(ui, cx, b.isolate);
}

pub fn traffic_lights(ui: &Ui, cx: &Cx) {
    let colours = [cx.t.traffic_close, cx.t.traffic_min, cx.t.traffic_max];
    for (i, c) in colours.into_iter().enumerate() {
        let centre = cx.lay.titlebar.left_center() + vec2(22.0 + 20.0 * i as f32, 0.0);
        ui.painter().circle_filled(centre, 6.0, c);
    }
}

fn title_bar(ui: &mut Ui, cx: &Cx) {
    at(ui, cx.lay.titlebar, |ui| {
        TitleBar::new("Mounting bracket")
            .dirty(true)
            .suffix(Some("Workshop"))
            .can_redo(false)
            .drag_window(false)
            .show(ui);
    });
    traffic_lights(ui, cx);
}

fn status_bar(ui: &mut Ui, cx: &Cx, s: &Status) {
    at(ui, cx.lay.statusbar, |ui| {
        StatusBar::new()
            .sketch(s.sketch)
            .dot(s.dot)
            .text(s.text)
            .hint(s.hint)
            .projection(s.persp)
            .items(s.items)
            .unit(Some("mm"))
            .show(ui);
    });
}

fn toggles(ui: &mut Ui, cx: &Cx, b: &Base) {
    let l = &cx.lay;
    let left = PanelToggle::new(Side::Left)
        .open(b.explorer.is_some())
        .tip(Tip::new("Explorer").shortcut(Some("⌘1")));
    ui.put(l.explorer_toggle, left);
    ui.put(
        l.workspace,
        WorkspaceButton::new("Design", "Workspace · ⌃1", "body"),
    );
    if b.inspector.is_none() {
        let right = PanelToggle::new(Side::Right)
            .dot(b.dot)
            .tip(Tip::new("Inspector").shortcut(Some("⌘2")));
        ui.put(l.inspector_toggle, right);
    }
}

fn view_cluster(ui: &mut Ui, cx: &Cx, sketch: bool) {
    let display = SquareButton {
        icon: "shadedEdges",
        on: false,
        enabled: true,
        tip: Tip::new("Display mode"),
    };
    let grid = SquareButton {
        icon: "grid",
        on: true,
        enabled: true,
        tip: Tip::new("Grid").shortcut(Some("G")),
    };
    let snap = SquareButton {
        icon: "snap",
        on: false,
        enabled: true,
        tip: Tip::new("Snap"),
    };
    let model = [display, grid, snap];
    let buttons = if sketch { &model[1..] } else { &model[..] };
    let rect = Rect::from_min_size(cx.lay.view_cluster, vec2(VIEW_CUBE, 300.0));
    at(ui, rect, |ui| {
        ViewCluster::new(buttons).show(ui, |ui, slot, _| {
            let style = IconStyle {
                tint: cx.t.ink4,
                bg: cx.t.canvas_model,
                grey: false,
            };
            icon::paint(
                ui.painter(),
                slot.shrink(4.0),
                "viewIso",
                cx.dark,
                style,
                1.0,
            );
        });
    });
}

fn nav_pill(ui: &mut Ui, cx: &Cx, isolate: bool) {
    let spec = [
        ("home", "Home", true),
        ("fit", "Fit", true),
        ("isolate", "Isolate", isolate),
        ("section", "Section", true),
        ("layers", "Layers", true),
    ];
    let buttons: Vec<NavButton> = spec
        .iter()
        .map(|(icon, name, enabled)| NavButton {
            icon,
            enabled: *enabled,
            active: false,
            tip: Tip::new(name).reason((!*enabled).then_some("Select a body first")),
        })
        .collect();
    let p: Pos2 = cx.lay.nav_pill;
    let rect = Rect::from_min_max(pos2(p.x, p.y - 80.0), pos2(p.x + 400.0, p.y));
    let layout = egui::Layout::bottom_up(egui::Align::Min);
    ui.scope_builder(UiBuilder::new().max_rect(rect).layout(layout), |ui| {
        NavPill::new(&buttons).show(ui);
    });
}

/// One pill button: icon, tooltip name, optional shortcut, state, disabled reason.
#[derive(Clone, Copy)]
pub struct Tool<'a> {
    icon: &'a str,
    name: &'a str,
    key: Option<&'a str>,
    state: ToolState,
    reason: Option<&'a str>,
}

impl<'a> Tool<'a> {
    pub fn new(icon: &'a str, name: &'a str) -> Self {
        Self {
            icon,
            name,
            key: None,
            state: ToolState::Default,
            reason: None,
        }
    }
    pub fn key(mut self, key: &'a str) -> Self {
        self.key = Some(key);
        self
    }
    pub fn state(mut self, state: ToolState) -> Self {
        self.state = state;
        self
    }
    pub fn off(mut self, reason: &'a str) -> Self {
        self.reason = Some(reason);
        self
    }
    fn add(&self, ui: &mut Ui) {
        let tip = Tip::new(self.name).shortcut(self.key).reason(self.reason);
        let b = ToolButton::new(self.icon)
            .state(self.state)
            .enabled(self.reason.is_none())
            .tip(tip);
        ui.add(b);
    }
}

pub struct PillSpec<'a> {
    pub mode: PillMode,
    pub accent: PillAccent,
    /// (kind, context, variant) of the SelectionLabel head.
    pub label: Option<(&'a str, &'a str, SelectionVariant)>,
    pub groups: &'a [&'a [Tool<'a>]],
}

/// The one toolbar, centred at the layout's pill centre.
pub fn pill(ui: &mut Ui, cx: &Cx, spec: &PillSpec) {
    centred(ui, cx.lay.pill_cx, cx.lay.pill_top, "pill", |ui| {
        ToolPill::new()
            .mode(spec.mode)
            .accent(spec.accent)
            .show(ui, |ui| {
                if let Some((kind, context, variant)) = spec.label {
                    SelectionLabel::new(kind, context).variant(variant).show(ui);
                    pill_divider(ui);
                }
                for (i, group) in spec.groups.iter().enumerate() {
                    if i > 0 {
                        pill_divider(ui);
                    }
                    group.iter().for_each(|tool| tool.add(ui));
                }
            });
    });
}

/// Session strip centred under the pill.
pub fn strip(
    ui: &mut Ui,
    cx: &Cx,
    variant: StripVariant,
    head: (&str, &str, Option<&str>),
    status: Option<(&str, Color32, Option<&str>)>,
    actions: &[StripAction],
) {
    let (icon, title, subtitle) = head;
    centred(ui, cx.lay.pill_cx, cx.lay.strip_top, "strip", |ui| {
        SessionStrip::new(variant)
            .icon(Some(icon))
            .title(title)
            .subtitle(subtitle)
            .status(status)
            .actions(actions)
            .show(ui);
    });
}

/// Build the chrome from `base`, then let the screen add its overlays.
pub fn screen<'a>(
    ui: &mut Ui,
    t: &'a Theme,
    dark: bool,
    stage: Rect,
    base: &Base,
    overlay: impl FnOnce(&mut Ui, &Cx<'a>),
) {
    let cx = Cx::new(t, dark, stage, base.flags());
    self::base(ui, &cx, base);
    overlay(ui, &cx);
}

/// `X`/`Y`/`Z` readout items in the status bar.
pub fn coords<'a>(x: &'a str, y: &'a str, z: Option<&'a str>) -> Vec<StatusItem<'a>> {
    let item = |label, value| StatusItem {
        label: Some(label),
        value,
        mono: true,
    };
    let mut v = vec![item("X", x), item("Y", y)];
    v.extend(z.map(|z| item("Z", z)));
    v
}
