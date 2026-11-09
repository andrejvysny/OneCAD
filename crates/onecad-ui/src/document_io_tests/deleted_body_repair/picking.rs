use super::*;

pub(super) async fn pick(
    backend: &mut Backend,
    session: &str,
    instance: OccurrenceId,
    cylinder: bool,
) -> (OriginPick, Frame) {
    let display = crate::capture::load_display(backend).await.unwrap();
    backend.shared.publish_display(display);
    let display = backend
        .shared
        .display
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .clone();
    let source_id = if cylinder { NEW_SOURCE } else { STOCK };
    let target = display
        .scene
        .bodies()
        .iter()
        .find(|body| {
            body.source_body_id().to_string() == source_id
                && body
                    .occurrence_path()
                    .and_then(|p| p.0.last())
                    .is_some_and(|id| id.0 == instance.0.as_u128())
        })
        .unwrap();
    let bounds = target.bounds();
    let camera = onecad_scene::Camera {
        target: (bounds.min + bounds.max) * 0.5,
        offset: onecad_scene::DVec3::new(75.0, -75.0, 100.0),
        ..Default::default()
    };
    let viewport = onecad_scene::Viewport {
        width: 800.0,
        height: 600.0,
        scale_factor: 1.0,
    };
    let view =
        onecad_scene::ViewSnapshot::new(display.scene.identity(), 1, camera, viewport).unwrap();
    let generation = display
        .assembly
        .definitions
        .iter()
        .find(|d| d.id == DEFINITION.to_string())
        .unwrap()
        .mesh_generation
        .unwrap();
    let path = OccurrencePath(vec![instance]);
    let mut seen = std::collections::BTreeSet::new();
    for x in (20..780).step_by(3) {
        for y in (20..580).step_by(3) {
            let hit = onecad_scene::pick_with_policy(
                &display.scene,
                &view,
                view.identity(),
                onecad_scene::DVec2::new(x as f64, y as f64),
                onecad_scene::PickPolicy::FacesAndEdges,
            )
            .unwrap();
            let Some(hit) = hit.filter(|hit| {
                hit.body_id == target.body_id()
                    && hit.kind == onecad_scene::PickKind::Face
                    && hit.ids_have_element_ids
            }) else {
                continue;
            };
            if !seen.insert(hit.topology_id.to_owned()) {
                continue;
            }
            let resolved = api::component_joint_origin(
                &backend.state,
                api::JointOriginRequest {
                    session,
                    generation: display.assembly.persisted_generation,
                    path: &path,
                    body: hit.source_body_id,
                    mesh_generation: generation,
                    element: hit.topology_id,
                    x: Vec3d::X,
                },
            )
            .await;
            let Ok(resolved) = resolved else { continue };
            let selection = crate::viewport_selection::OccurrenceSelection::from_hit(&hit).unwrap();
            return (
                OriginPick {
                    side: JointSide::A,
                    selection,
                    mesh_generation: generation,
                },
                resolved.frame,
            );
        }
    }
    panic!("fresh occurrence face not pickable: {instance}, cylinder={cylinder}");
}
