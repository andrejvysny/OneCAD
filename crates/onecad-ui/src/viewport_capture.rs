//! A redraw does not retire a pick when its geometry and presentation are unchanged.
use onecad_render::{PreviewIdentity, RenderOptions};
use onecad_scene::{Camera, SceneIdentity, ViewSnapshot, Viewport};

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Presentation {
    pub options: RenderOptions,
    pub dark: bool,
    pub preview: Option<PreviewIdentity>,
}

pub(crate) struct Request {
    pub scene: SceneIdentity,
    pub camera: Camera,
    pub viewport: Viewport,
    pub presentation: Presentation,
}

pub(crate) fn next(
    previous: Option<ViewSnapshot>,
    previous_presentation: Option<Presentation>,
    request: Request,
    sequence: &mut u64,
) -> Result<ViewSnapshot, String> {
    if let Some(view) = previous.filter(|view| {
        view.identity().scene == request.scene
            && view.camera() == request.camera
            && view.viewport() == request.viewport
            && previous_presentation == Some(request.presentation)
    }) {
        return Ok(view);
    }
    let number = sequence
        .checked_add(1)
        .ok_or("Viewport capture sequence exhausted")?;
    let view = ViewSnapshot::new(request.scene, number, request.camera, request.viewport)
        .map_err(|error| error.to_string())?;
    *sequence = number;
    Ok(view)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> Request {
        Request {
            scene: SceneIdentity {
                session: 1,
                scene: 1,
            },
            camera: Default::default(),
            viewport: Viewport {
                width: 800.0,
                height: 600.0,
                scale_factor: 2.0,
            },
            presentation: Presentation {
                options: Default::default(),
                dark: true,
                preview: None,
            },
        }
    }

    #[test]
    fn repeated_drawing_retains_capture_but_real_geometry_and_visibility_changes_retire_it() {
        let mut sequence = 0;
        let first = next(None, None, request(), &mut sequence).unwrap();
        let style = request().presentation;
        for _ in 0..4 {
            assert_eq!(
                next(Some(first), Some(style), request(), &mut sequence)
                    .unwrap()
                    .identity(),
                first.identity()
            );
        }
        assert_eq!(sequence, 1);
        for change in 0..7 {
            let mut changed = request();
            match change {
                0 => changed.camera.target.x += 1.0,
                1 => changed.viewport.width += 1.0,
                2 => changed.viewport.scale_factor = 1.0,
                3 => changed.scene.scene += 1,
                4 => {
                    changed.presentation.options.display_mode =
                        onecad_render::DisplayMode::Wireframe
                }
                5 => changed.presentation.dark = false,
                _ => {
                    changed.presentation.preview = Some(PreviewIdentity {
                        base: changed.scene,
                        generation: 1,
                    })
                }
            }
            let result = next(Some(first), Some(style), changed, &mut sequence).unwrap();
            assert_ne!(result.identity(), first.identity());
        }
    }

    #[test]
    fn invalid_or_exhausted_new_capture_never_advances_sequence() {
        let mut sequence = u64::MAX;
        assert!(next(None, None, request(), &mut sequence).is_err());
        assert_eq!(sequence, u64::MAX);
        sequence = 7;
        let mut bad = request();
        bad.viewport.width = 0.0;
        assert!(next(None, None, bad, &mut sequence).is_err());
        assert_eq!(sequence, 7);
    }
}
