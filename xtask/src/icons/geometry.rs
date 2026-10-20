use super::{Glyph, Item, Point};

pub(super) fn project(glyph: &Glyph, compact: bool) -> (Vec<Item>, f64) {
    let mut items: Vec<_> = glyph
        .items
        .iter()
        .filter(|i| !(compact && i.o.detail))
        .cloned()
        .collect();
    let mut bounds = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    let mut any = false;
    for i in items.iter_mut().filter(|i| i.d3) {
        any = true;
        map(i, |p| {
            vec![(p[0] - p[1]) * 0.9, (p[0] + p[1]) * 0.42 - p[2]]
        });
        for p in i.pts.iter().chain(i.holes.iter().flatten()) {
            bounds[0] = bounds[0].min(p[0]);
            bounds[1] = bounds[1].min(p[1]);
            bounds[2] = bounds[2].max(p[0]);
            bounds[3] = bounds[3].max(p[1]);
        }
    }
    if !any {
        return (items, 1.0);
    }
    let fit = if compact && glyph.fit == [2.0, 2.0, 22.0, 22.0] {
        [1.5, 1.5, 22.5, 22.5]
    } else {
        glyph.fit
    };
    let scale = (if compact { 99.0_f64 } else { 0.92 })
        .min((fit[2] - fit[0]) / (bounds[2] - bounds[0]))
        .min((fit[3] - fit[1]) / (bounds[3] - bounds[1]));
    let cx = (fit[0] + fit[2]) / 2.0;
    let cy = (fit[1] + fit[3]) / 2.0;
    let mx = (bounds[0] + bounds[2]) / 2.0;
    let my = (bounds[1] + bounds[3]) / 2.0;
    for i in items.iter_mut().filter(|i| i.d3) {
        map(i, |p| {
            vec![cx + (p[0] - mx) * scale, cy + (p[1] - my) * scale]
        });
    }
    (items, scale)
}

fn map(item: &mut Item, f: impl Fn(&Point) -> Point) {
    item.pts = item.pts.iter().map(&f).collect();
    item.holes = item
        .holes
        .iter()
        .map(|h| h.iter().map(&f).collect())
        .collect();
    item.o.prepend = item.o.prepend.iter().map(f).collect();
}

pub(super) fn num(n: f64) -> String {
    if n == 0.0 {
        return "0".into();
    }
    ryu_js::Buffer::new().format(n).to_string()
}

fn rounded(n: f64) -> String {
    num((n * 100.0 + 0.5).floor() / 100.0)
}

pub(super) fn poly(points: &[Point], close: bool) -> String {
    let mut out = String::new();
    for (i, p) in points.iter().enumerate() {
        out.push(if i == 0 { 'M' } else { 'L' });
        out.push_str(&rounded(p[0]));
        out.push(' ');
        out.push_str(&rounded(p[1]));
    }
    if close {
        out.push('Z');
    }
    out
}

pub(super) fn circle(p: &Point, r: f64) -> String {
    format!(
        "M{} {}a{} {} 0 1 0 {} 0a{} {} 0 1 0 {} 0Z",
        rounded(p[0] - r),
        rounded(p[1]),
        rounded(r),
        rounded(r),
        rounded(2.0 * r),
        rounded(r),
        rounded(r),
        rounded(-2.0 * r)
    )
}

pub(super) fn arrow(points: &mut [Point], end: bool, w: f64) -> String {
    let n = points.len();
    let at = if end { n - 1 } else { 0 };
    let e = points[at].clone();
    let mut j = if end { n - 2 } else { 1 };
    while (e[0] - points[j][0]).hypot(e[1] - points[j][1]) < 1.0 && j > 0 && j < n - 1 {
        if end {
            j -= 1;
        } else {
            j += 1;
        }
    }
    let ux = e[0] - points[j][0];
    let uy = e[1] - points[j][1];
    let length = ux.hypot(uy);
    let length = if length == 0.0 { 1.0 } else { length };
    let ux = ux / length;
    let uy = uy / length;
    let h = 2.9_f64.max(w * 2.05);
    let hw = 1.95_f64.max(w * 1.4);
    let bx = e[0] - ux * h;
    let by = e[1] - uy * h;
    let path = poly(
        &[
            e.clone(),
            vec![bx - uy * hw, by + ux * hw],
            vec![bx + uy * hw, by - ux * hw],
        ],
        true,
    );
    points[at] = vec![e[0] - ux * h * 0.7, e[1] - uy * h * 0.7];
    path
}
