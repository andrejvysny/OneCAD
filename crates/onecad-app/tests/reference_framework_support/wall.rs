use super::*;

/// Diagnostic straight portion of source wall #121 and adjacent planes #88/#90.
pub(crate) async fn cut_straight_wall(h: &Harness, body: BodyId, side: usize, n: u128) {
    let sign = if side == 0 { -1.0 } else { 1.0 };
    let points = [
        [14.19, 3.942260869565],
        [15.1, 2.984782608696546],
        [15.1, 4.30524077792813],
        [14.19, 4.25754969878],
    ]
    .map(|[x, z]| [-sign * x, z]);
    let start = 2.203927026704;
    let end = 26.8;
    let profile = super::slot_profile(h, n, start, WorldPlane::YZ, &points).await;
    assert_eq!(profile.regions.len(), 1);
    super::extrusion(h, n, &profile, end - start, Some(body)).await;
}
