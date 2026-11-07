use std::f64::consts::PI;

pub struct Geometry {
    pub center: [f64; 3],
    pub major: [f64; 3],
    pub minor: [f64; 3],
    pub normal: [f64; 3],
    pub profile_center: [f64; 3],
    pub profile_x: [f64; 3],
    pub profile_y: [f64; 3],
    pub profile_normal: [f64; 3],
    pub a: f64,
    pub b: f64,
    pub start: f64,
    pub end: f64,
}

pub fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    (0..3).map(|i| a[i] * b[i]).sum()
}
fn sample(g: &Geometry, t: f64) -> ([f64; 3], [f64; 3], f64) {
    let (sin, cos) = t.sin_cos();
    let p = std::array::from_fn(|i| g.center[i] + g.a * cos * g.major[i] + g.b * sin * g.minor[i]);
    let derivative = std::array::from_fn(|i| -g.a * sin * g.major[i] + g.b * cos * g.minor[i]);
    let speed = dot(derivative, derivative).sqrt();
    (p, derivative.map(|v| v / speed), speed)
}
pub fn expected(g: &Geometry, radius: f64, end: f64) -> (f64, [f64; 3]) {
    let n = 16384;
    let h = (end - g.start) / n as f64;
    let mut integral = 0.;
    let mut moment = [0.; 3];
    for i in 0..=n {
        let (point, _, speed) = sample(g, g.start + i as f64 * h);
        let weight = if i == 0 || i == n {
            1.
        } else if i % 2 == 0 {
            2.
        } else {
            4.
        };
        integral += weight * speed;
        for axis in 0..3 {
            moment[axis] += weight * speed * point[axis];
        }
    }
    let length = integral * h / 3.;
    let (_, first, _) = sample(g, g.start);
    let (_, last, _) = sample(g, end);
    // Curvature weights the inner side of each disc by 1 − κu, so its first
    // moment needs the tangent-end correction rather than a centerline average.
    let centroid = std::array::from_fn(|i| {
        (moment[i] * h / 3. - radius * radius * (last[i] - first[i]) / 4.) / length
    });
    (PI * radius * radius * length, centroid)
}
pub fn bounds(g: &Geometry, radius: f64, end: f64) -> ([f64; 3], [f64; 3]) {
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    for i in 0..=16384 {
        let (center, tangent, _) = sample(g, g.start + (end - g.start) * i as f64 / 16384.);
        for axis in 0..3 {
            let extent = radius * (1. - tangent[axis] * tangent[axis]).max(0.).sqrt();
            lo[axis] = lo[axis].min(center[axis] - extent);
            hi[axis] = hi[axis].max(center[axis] + extent);
        }
    }
    (lo, hi)
}
