//! Immediate wheel extraction adapted from SolidCraft's navigation module.
//! Events retain their order; OneCAD's camera owns application of each motion.

use egui::{Event, MouseWheelSource, MouseWheelUnit, TouchPhase, Vec2};

const GESTURE_IDLE_SECONDS: f64 = 0.120;

#[derive(Debug, PartialEq)]
pub enum Motion {
    Pan(Vec2),
    Orbit(Vec2),
    Zoom(f64),
}

#[derive(Default)]
pub struct Navigation {
    pad: Option<Pad>,
    unknown_bracket: bool,
}

struct Pad {
    shift: bool,
    last: f64,
    ended: bool,
}

impl Navigation {
    pub fn event(&mut self, event: &Event, time: f64, page_height: f32) -> Option<Motion> {
        if matches!(event, Event::WindowFocused(false) | Event::PointerGone)
            || !time.is_finite()
            || time < 0.0
            || !page_height.is_finite()
            || page_height <= 0.0
        {
            *self = Self::default();
            return None;
        }
        match event {
            Event::Zoom(factor) if factor.is_finite() && *factor > 0.0 => {
                Some(Motion::Zoom((1.0 / f64::from(*factor)).clamp(0.2, 5.0)))
            }
            Event::MouseWheel {
                unit,
                delta,
                modifiers,
                phase,
                source,
            } => {
                if !delta.is_finite() {
                    *self = Self::default();
                    return None;
                }
                let pad = self.trackpad(*source, *unit, *phase);
                if *phase == TouchPhase::Cancel {
                    return None;
                }
                if pad && self.pad.as_ref().is_some_and(|pad| time < pad.last) {
                    *self = Self::default();
                    return None;
                }
                let shift = pad && self.latch(*source, *phase, modifiers.shift, time);
                motion(
                    *delta,
                    wheel_scale(*unit, page_height),
                    pad,
                    shift,
                    modifiers.ctrl,
                )
            }
            _ => None,
        }
    }

    fn trackpad(
        &mut self,
        source: MouseWheelSource,
        unit: MouseWheelUnit,
        phase: TouchPhase,
    ) -> bool {
        if source == MouseWheelSource::Wheel {
            return false;
        }
        if phase == TouchPhase::Cancel {
            *self = Self::default();
            return false;
        }
        if source != MouseWheelSource::Unknown {
            return true;
        }
        if unit != MouseWheelUnit::Point {
            return false;
        }
        if phase == TouchPhase::Start {
            self.unknown_bracket = true;
        }
        let bracket = self.unknown_bracket;
        if phase == TouchPhase::End {
            self.unknown_bracket = false;
        }
        bracket
    }

    fn latch(
        &mut self,
        source: MouseWheelSource,
        phase: TouchPhase,
        shift: bool,
        time: f64,
    ) -> bool {
        let recent = self
            .pad
            .as_ref()
            .is_some_and(|pad| time >= pad.last && time - pad.last <= GESTURE_IDLE_SECONDS);
        let momentum = source == MouseWheelSource::Momentum
            && recent
            && self.pad.as_ref().is_some_and(|pad| pad.ended);
        let fresh = !recent || (phase == TouchPhase::Start && !momentum);
        if fresh {
            self.pad = Some(Pad {
                shift,
                last: time,
                ended: false,
            });
        }
        let pad = self.pad.get_or_insert(Pad {
            shift,
            last: time,
            ended: false,
        });
        pad.last = time;
        pad.ended = phase == TouchPhase::End;
        pad.shift
    }
}

fn wheel_scale(unit: MouseWheelUnit, page_height: f32) -> f64 {
    match unit {
        MouseWheelUnit::Point => 1.0,
        MouseWheelUnit::Line => 16.0,
        MouseWheelUnit::Page => f64::from(page_height),
    }
}

fn motion(delta: Vec2, scale: f64, pad: bool, shift: bool, ctrl: bool) -> Option<Motion> {
    let x = f64::from(delta.x) * scale;
    let y = f64::from(delta.y) * scale;
    if ctrl || !pad {
        let axis = if y == 0.0 { x } else { y };
        if axis == 0.0 {
            return None;
        }
        let sensitivity = if ctrl { 0.01 } else { 0.0015 };
        // Clamp the exponent before exp so finite extreme input cannot overflow.
        return Some(Motion::Zoom(
            (-axis * sensitivity)
                .clamp(0.2_f64.ln(), 5.0_f64.ln())
                .exp(),
        ));
    }
    let gain = if shift { 0.6 } else { 1.0 };
    let delta = egui::vec2((x * gain) as f32, (y * gain) as f32);
    if !delta.is_finite() || delta == Vec2::ZERO {
        return None;
    }
    Some(if shift {
        Motion::Orbit(delta)
    } else {
        Motion::Pan(delta)
    })
}

#[cfg(test)]
#[path = "navigation_tests.rs"]
mod tests;
