use super::*;

pub(crate) async fn initial_body(
    h: &Harness,
    recipe: &Value,
) -> (BodyId, onecad_app::dto::MassPropertiesDto) {
    let stock = json!([[{"kind":"line","start":[-15.0,-32.0],"end":[15.0,-32.0]}, {"kind":"line","start":[15.0,-32.0],"end":[15.0,0.0]}, {"kind":"line","start":[15.0,0.0],"end":[-15.0,0.0]}, {"kind":"line","start":[-15.0,0.0],"end":[-15.0,-32.0]}]]);
    let profile = add_sketch(h, 1, &stock, None).await;
    assert_eq!(profile.regions.len(), 1);
    extrusion(h, 1, &profile, 6.8, None).await;
    let projection = h.projection().await;
    assert_eq!(projection.bodies.len(), 1);
    let body: BodyId = projection.bodies.keys().next().unwrap().parse().unwrap();
    let wire = onecad_app::worker::wire::body_id_wire(body);
    let stock_mass = api::query_mass_properties(&h.state, wire.clone(), None)
        .await
        .unwrap();
    assert!((stock_mass.volume - 6528.0).abs() < 1e-8);
    let profile = add_sketch(h, 2, &recipe["interiorFloorFace"]["wires"], Some(6.8)).await;
    assert_eq!(profile.regions.len(), 1);
    extrusion(h, 2, &profile, -5.3, Some(body)).await;
    let mass = api::query_mass_properties(&h.state, wire, None)
        .await
        .unwrap();
    eprintln!("Framework stock/pocket diagnostic volume {} mm³; area {} mm²; source enclosure OCP volume 2002.3764093378575 mm³", mass.volume, mass.surface_area);
    assert!(mass.volume < stock_mass.volume);
    for index in 0..2 {
        let outer = json!([recipe["bossTopFaces"][index]["wires"][0]]);
        let boss_profile = add_sketch(h, 3 + index as u128, &outer, Some(6.8)).await;
        assert_eq!(boss_profile.regions.len(), 1);
        extrusion(h, 3 + index as u128, &boss_profile, -3.7, Some(body)).await;
    }
    let wire = onecad_app::worker::wire::body_id_wire(body);
    let boss_mass = api::query_mass_properties(&h.state, wire, None)
        .await
        .unwrap();
    eprintln!(
        "Framework stock, pocket and boss caps volume {} mm³, area {} mm²",
        boss_mass.volume, boss_mass.surface_area
    );
    assert!(boss_mass.volume < mass.volume);
    for (index, source_x) in [-11.3, 11.3].into_iter().enumerate() {
        super::drill_boss(h, body, source_x, 10 + index as u128).await;
    }
    let hole_mass =
        api::query_mass_properties(&h.state, onecad_app::worker::wire::body_id_wire(body), None)
            .await
            .unwrap();
    let r = 1.65_f64;
    let big_r = 1.85_f64;
    let removed = 2.0
        * std::f64::consts::PI
        * (2.5 * r * r + 0.2 * (big_r * big_r + big_r * r - 2.0 * r * r) / 3.0);
    assert!((boss_mass.volume - hole_mass.volume - removed).abs() < 1e-7);
    eprintln!(
        "Framework intermediate with boss holes volume {} mm³, area {} mm²",
        hole_mass.volume, hole_mass.surface_area
    );
    super::fillet_lower_front(h, body).await;
    let fillet_mass =
        api::query_mass_properties(&h.state, onecad_app::worker::wire::body_id_wire(body), None)
            .await
            .unwrap();
    let removed = 30.0 * 0.8_f64.powi(2) * (1.0 - std::f64::consts::PI / 4.0);
    assert!((hole_mass.volume - fillet_mass.volume - removed).abs() < 1e-7);
    eprintln!(
        "Framework intermediate with source lower front radius volume {} mm³, area {} mm²",
        fillet_mass.volume, fillet_mass.surface_area
    );
    super::fillet_upper_outer(h, body).await;
    let upper_mass =
        api::query_mass_properties(&h.state, onecad_app::worker::wire::body_id_wire(body), None)
            .await
            .unwrap();
    assert!(upper_mass.volume < fillet_mass.volume);
    eprintln!(
        "Framework intermediate with outer upper radii volume {} mm³, area {} mm²",
        upper_mass.volume, upper_mass.surface_area
    );
    (body, upper_mass)
}
