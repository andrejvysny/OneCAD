//! Origin axis triad. Drawn in the main pass with the edge depth test so solids occlude it;
//! a 2D overlay cannot know which parts of the rays are hidden.
use crate::frame::{rgba, token, FrameU};
use crate::{RenderError, Theme};
use onecad_scene::{DVec3, ViewSnapshot};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AxisColor {
    X,
    Y,
    Z,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AxisLine {
    pub direction: DVec3,
    pub color: AxisColor,
}

/// Positive half-rays from one origin, each with an origin dot.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AxesFrame {
    pub origin: DVec3,
    pub lines: [Option<AxisLine>; 3],
}

const WIDTH: f32 = 1.25;
const DOT: f32 = 5.0;
const ALPHA: f32 = 0.75;

pub(crate) fn set_axes(
    frame: &mut FrameU,
    view: &ViewSnapshot,
    axes: Option<AxesFrame>,
    theme: Theme,
) -> Result<(), RenderError> {
    let Some(axes) = axes.filter(|axes| axes.origin.is_finite()) else {
        return Ok(());
    };
    // Long enough to leave the far plane from any eye; the line shader clips to near/far.
    let length = view.camera().far + (view.eye() - axes.origin).length();
    frame.axes_origin = axes.origin.as_vec3().extend(length as f32).to_array();
    let scale = view.viewport().scale_factor as f32;
    frame.axes_style = [WIDTH * scale, DOT * scale, 1.0, 0.0];
    for (slot, line) in axes.lines.iter().enumerate() {
        let Some(line) = line else {
            continue;
        };
        let Some(direction) = line.direction.try_normalize() else {
            continue;
        };
        frame.axes_dir[slot] = direction.as_vec3().extend(1.0).to_array();
        frame.axes_color[slot] = rgba(token(theme, token_name(line.color))?, ALPHA);
    }
    Ok(())
}

fn token_name(color: AxisColor) -> &'static str {
    match color {
        AxisColor::X => "--color-axis-x",
        AxisColor::Y => "--color-axis-y",
        AxisColor::Z => "--color-axis-z",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BodyStyle, RenderStyle};
    use onecad_scene::{Camera, SceneIdentity, Viewport};

    #[test]
    fn frames_with_axes_resolve_in_both_themes() {
        let view = ViewSnapshot::new(
            SceneIdentity {
                session: 1,
                scene: 1,
            },
            1,
            Camera::default(),
            Viewport {
                width: 800.0,
                height: 600.0,
                scale_factor: 2.0,
            },
        )
        .unwrap();
        let line = |direction, color| Some(AxisLine { direction, color });
        let axes = AxesFrame {
            origin: DVec3::ZERO,
            lines: [
                line(DVec3::X, AxisColor::X),
                line(DVec3::Y, AxisColor::Y),
                line(DVec3::Z, AxisColor::Z),
            ],
        };
        for theme in [Theme::Light, Theme::Dark] {
            let style = RenderStyle {
                theme,
                axes: Some(axes),
                ..Default::default()
            };
            let frame =
                crate::frame::uniform(&view, None, style, BodyStyle::default(), false).unwrap();
            assert_eq!(frame.axes_style[2], 1.0);
            assert!(frame.axes_dir.iter().all(|dir| dir[3] == 1.0));
            assert!(frame.axes_color.iter().all(|color| color[3] == ALPHA));
        }
    }
}
