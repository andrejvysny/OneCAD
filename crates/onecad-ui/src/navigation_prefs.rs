//! Persisted mouse navigation mapping and wheel behaviour; pure data and rules.
use crate::navigation::Motion;
use egui::PointerButton;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum NavPreset {
    ThreeButtonMouse,
    Trackpad,
    Custom,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
#[allow(
    clippy::enum_variant_names,
    reason = "Variant names are the persisted camelCase spellings"
)]
pub enum DragInput {
    RightDrag,
    ShiftRightDrag,
    MiddleDrag,
    ShiftMiddleDrag,
}

impl DragInput {
    pub const ALL: [DragInput; 4] = [
        Self::RightDrag,
        Self::ShiftRightDrag,
        Self::MiddleDrag,
        Self::ShiftMiddleDrag,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::RightDrag => "Right drag",
            Self::ShiftRightDrag => "⇧ Right drag",
            Self::MiddleDrag => "Middle drag",
            Self::ShiftMiddleDrag => "⇧ Middle drag",
        }
    }

    fn parts(self) -> (PointerButton, bool) {
        match self {
            Self::RightDrag => (PointerButton::Secondary, false),
            Self::ShiftRightDrag => (PointerButton::Secondary, true),
            Self::MiddleDrag => (PointerButton::Middle, false),
            Self::ShiftMiddleDrag => (PointerButton::Middle, true),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DragAction {
    Orbit,
    Pan,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MappingSlot {
    Orbit,
    Pan,
    PanAlt,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct NavigationPrefs {
    pub preset: NavPreset,
    pub orbit: DragInput,
    pub pan: DragInput,
    pub pan_alt: Option<DragInput>,
    pub fit_on_middle_double_click: bool,
    pub zoom_toward_cursor: bool,
    pub invert_scroll_zoom: bool,
}

impl Default for NavigationPrefs {
    fn default() -> Self {
        Self {
            preset: NavPreset::ThreeButtonMouse,
            orbit: DragInput::RightDrag,
            pan: DragInput::MiddleDrag,
            pan_alt: Some(DragInput::ShiftRightDrag),
            fit_on_middle_double_click: true,
            zoom_toward_cursor: true,
            invert_scroll_zoom: false,
        }
    }
}

impl NavigationPrefs {
    pub fn apply_preset(&mut self, preset: NavPreset) {
        let (orbit, pan, pan_alt, fit) = match preset {
            NavPreset::ThreeButtonMouse => (
                DragInput::RightDrag,
                DragInput::MiddleDrag,
                Some(DragInput::ShiftRightDrag),
                true,
            ),
            NavPreset::Trackpad => (DragInput::RightDrag, DragInput::ShiftRightDrag, None, false),
            NavPreset::Custom => {
                self.preset = preset;
                return;
            }
        };
        self.preset = preset;
        self.orbit = orbit;
        self.pan = pan;
        self.pan_alt = pan_alt;
        self.fit_on_middle_double_click = fit;
    }

    pub fn reset_to_preset(&mut self) {
        let preset = match self.preset {
            NavPreset::Custom => NavPreset::ThreeButtonMouse,
            other => other,
        };
        self.apply_preset(preset);
    }

    fn slot(&self, slot: MappingSlot) -> Option<DragInput> {
        match slot {
            MappingSlot::Orbit => Some(self.orbit),
            MappingSlot::Pan => Some(self.pan),
            MappingSlot::PanAlt => self.pan_alt,
        }
    }

    fn set_slot(&mut self, slot: MappingSlot, input: Option<DragInput>) {
        match (slot, input) {
            (MappingSlot::Orbit, Some(i)) => self.orbit = i,
            (MappingSlot::Pan, Some(i)) => self.pan = i,
            (MappingSlot::PanAlt, i) => self.pan_alt = i,
            _ => {}
        }
    }

    /// Returns whether anything changed; any change leaves the preset as `Custom`.
    pub fn assign(&mut self, slot: MappingSlot, input: Option<DragInput>) -> bool {
        let Some(input) = input else {
            if slot == MappingSlot::PanAlt && self.pan_alt.is_some() {
                self.pan_alt = None;
                self.preset = NavPreset::Custom;
                return true;
            }
            return false;
        };
        if self.slot(slot) == Some(input) {
            return false;
        }
        let holder = [MappingSlot::Orbit, MappingSlot::Pan, MappingSlot::PanAlt]
            .into_iter()
            .find(|other| *other != slot && self.slot(*other) == Some(input));
        match (slot, holder) {
            (MappingSlot::PanAlt, Some(_)) => return false,
            (_, Some(MappingSlot::PanAlt)) => self.pan_alt = None,
            (_, Some(other)) => self.set_slot(other, self.slot(slot)),
            (_, None) => {}
        }
        self.set_slot(slot, Some(input));
        self.preset = NavPreset::Custom;
        true
    }

    pub fn set_fit_on_middle_double_click(&mut self, on: bool) {
        if self.fit_on_middle_double_click != on {
            self.fit_on_middle_double_click = on;
            self.preset = NavPreset::Custom;
        }
    }

    pub fn drag_action(&self, button: PointerButton, shift: bool) -> Option<DragAction> {
        let slots = [
            (Some(self.orbit), DragAction::Orbit),
            (Some(self.pan), DragAction::Pan),
            (self.pan_alt, DragAction::Pan),
        ];
        let find = |shift: bool| {
            slots
                .iter()
                .find(|(input, _)| input.is_some_and(|i| i.parts() == (button, shift)))
                .map(|(_, action)| *action)
        };
        // A held modifier the user did not map still drags with the bare button.
        find(shift).or_else(|| if shift { find(false) } else { None })
    }

    pub fn validate(&self) -> Result<(), &'static str> {
        if self.orbit == self.pan {
            return Err("orbit and pan need different mouse inputs");
        }
        if self
            .pan_alt
            .is_some_and(|alt| alt == self.orbit || alt == self.pan)
        {
            return Err("alternate pan must differ from orbit and pan");
        }
        Ok(())
    }
}

/// Motion for the buttons currently dragging; both held keeps the historic pan.
pub fn drag_motion(
    prefs: &NavigationPrefs,
    secondary: bool,
    middle: bool,
    shift: bool,
    delta: egui::Vec2,
) -> Option<Motion> {
    let button = match (secondary, middle) {
        (true, false) => PointerButton::Secondary,
        (false, true) => PointerButton::Middle,
        (true, true) => return Some(Motion::Pan(delta)),
        (false, false) => return None,
    };
    match prefs.drag_action(button, shift)? {
        DragAction::Orbit => Some(Motion::Orbit(delta)),
        DragAction::Pan => Some(Motion::Pan(delta)),
    }
}

/// Only wheel/trackpad zoom inverts; a pinch keeps its physical direction.
pub fn wheel_motion(prefs: &NavigationPrefs, wheel: bool, motion: Motion) -> Motion {
    match motion {
        Motion::Zoom(factor) if wheel && prefs.invert_scroll_zoom => Motion::Zoom(1.0 / factor),
        other => other,
    }
}

pub fn zoom_anchor<T>(prefs: &NavigationPrefs, cursor: Option<T>) -> Option<T> {
    cursor.filter(|_| prefs.zoom_toward_cursor)
}

#[cfg(test)]
#[path = "navigation_prefs_tests.rs"]
mod tests;
