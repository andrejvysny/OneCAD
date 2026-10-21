use super::{
    geometry::{arrow, circle, num, poly, project},
    Glyph, Item, Value, GAP,
};
use std::collections::BTreeMap;
type Palette = BTreeMap<String, Value>;

struct Path {
    d: String,
    fill: String,
    fill_opacity: f64,
    stroke: String,
    stroke_opacity: f64,
    width: f64,
    dash: &'static str,
}
impl Path {
    fn new(d: String) -> Self {
        Self {
            d,
            fill: "none".into(),
            fill_opacity: 1.0,
            stroke: "none".into(),
            stroke_opacity: 1.0,
            width: 0.0,
            dash: "none",
        }
    }
    fn svg(&self) -> String {
        let dash = if self.dash == "none" {
            String::new()
        } else {
            format!(" stroke-dasharray=\"{}\"", self.dash)
        };
        format!("<path d=\"{}\" fill=\"{}\" fill-opacity=\"{}\" fill-rule=\"evenodd\" stroke=\"{}\" stroke-opacity=\"{}\" stroke-width=\"{}\"{dash}/>",
            self.d, self.fill, num(self.fill_opacity), self.stroke, num(self.stroke_opacity), num(self.width))
    }
}

fn color(p: &Palette, key: &str, shade: &str) -> String {
    match p.get(key) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(a)) => a[match shade {
            "left" => 1,
            "right" => 2,
            _ => 0,
        }]
        .as_str()
        .unwrap()
        .into(),
        _ => "none".into(),
    }
}

pub(super) fn render(glyph: &Glyph, palette: &Palette, theme: &str, size: u32) -> String {
    let (items, scale) = project(glyph, size <= 20);
    let ow = 1.0_f64.max(f64::from(size) / 32.0) * (24.0 / f64::from(size));
    let lw = match size {
        0..=16 => 2.05,
        17..=20 => 1.85,
        21..=24 => 1.65,
        _ => 1.45,
    };
    let mut out = format!("<svg width=\"{size}\" height=\"{size}\" viewBox=\"0 0 24 24\" fill=\"none\" stroke-linejoin=\"round\" stroke-linecap=\"round\" style=\"display:block;overflow:visible\" aria-hidden=\"true\">");
    for item in items {
        let paths = match item.t.as_str() {
            "f" => vec![face(&item, palette, ow, lw)],
            "tube" => tube(&item, palette, ow, scale),
            "l" => line(&item, palette, lw, size <= 16),
            "m" => marker(&item, palette, theme, ow, size <= 16),
            _ => unreachable!("unknown icon geometry item"),
        };
        for path in paths {
            if !path.d.is_empty() {
                out.push_str(&path.svg());
            }
        }
    }
    out.push_str("</svg>");
    out
}

fn face(i: &Item, p: &Palette, ow: f64, lw: f64) -> Path {
    let mut d = poly(&i.pts, true);
    for h in &i.holes {
        d.push_str(&poly(h, true));
    }
    let mut s = Path::new(d);
    s.fill = color(p, &i.fam, &i.sh);
    s.fill_opacity = i.o.op.unwrap_or(1.0);
    s.stroke = if i.o.ns {
        "none".into()
    } else {
        color(p, "outline", "")
    };
    s.stroke_opacity = i.o.so.unwrap_or(1.0);
    s.width = ow;
    match i.fam.as_str() {
        "q" | "p" => {
            let key = if i.fam == "q" { "m" } else { "n" };
            s.fill = color(p, key, "left");
            s.stroke = color(p, key, "right");
            s.fill_opacity = i.o.op.unwrap_or(if i.fam == "q" { 0.26 } else { 0.42 });
        }
        "ink" => {
            s.fill = "currentColor".into();
            s.stroke = "none".into();
        }
        "none" => {
            s.fill = "none".into();
            s.stroke = "currentColor".into();
            s.stroke_opacity = 0.7;
            s.dash = "1.7 1.5";
            s.width = lw * 0.75;
        }
        _ => {}
    }
    s
}

fn tube(i: &Item, p: &Palette, ow: f64, scale: f64) -> Vec<Path> {
    let points: Vec<_> = i.o.prepend.iter().chain(&i.pts).cloned().collect();
    let width = if i.d3 { i.w * scale } else { i.w };
    let mut outer = Path::new(poly(&points, false));
    outer.stroke = color(p, "outline", "");
    outer.width = width + 2.0 * ow;
    let mut inner = Path::new(outer.d.clone());
    inner.stroke = color(p, &i.fam, &i.sh);
    inner.width = width;
    vec![outer, inner]
}

fn line_color(key: &str, p: &Palette) -> (String, f64) {
    let (key, opacity) = match key {
        "outline" | "tubeInk" => ("outline", 1.0),
        "thin" => ("currentColor", 0.55),
        "hid" => ("currentColor", 0.6),
        "none" => ("none", 0.0),
        "sk" | "mod" | "axis" | "az" => ("blue", 1.0),
        "skc" => ("blue", 0.85),
        "cons" | "new" => ("orange", 1.0),
        "con" | "cond" | "ax" => ("red", 1.0),
        "sel" | "ay" => ("green", 1.0),
        "white" => ("#ffffff", 1.0),
        "ghost" => ("blue", 0.95),
        "thinB" => ("m", 0.9),
        "thinO" => ("outline", 0.6),
        "dots" => ("currentColor", 0.75),
        "hatch" => ("outline", 0.55),
        "amber" => ("amber", 1.0),
        "gap" => (GAP, 1.0),
        _ => ("currentColor", 1.0),
    };
    (
        if key.starts_with('#') || key == "currentColor" || key == "none" {
            key.into()
        } else {
            color(p, key, "right")
        },
        opacity,
    )
}

fn dash(key: &str) -> &'static str {
    match key {
        "dots" => "0.01 3.4",
        "inkd" => "2.4 1.9",
        "hid" => "1.7 1.5",
        "skc" | "cond" => "2 1.7",
        "cons" => "2.4 1.6",
        "axis" => "3.6 1.3 0.1 1.3",
        "ghost" => "1.6 1.4",
        _ => "none",
    }
}
fn weight(key: &str) -> f64 {
    match key {
        "dots" => 1.15,
        "thinO" | "thin" => 0.65,
        "hid" => 0.75,
        "skc" => 0.85,
        "axis" | "ghost" => 0.8,
        "thinB" => 0.55,
        "hatch" => 0.5,
        _ => 1.0,
    }
}

fn line(i: &Item, p: &Palette, lw: f64, small: bool) -> Vec<Path> {
    let (default_color, opacity) = line_color(&i.k, p);
    let stroke_color = i.o.fixed.clone().unwrap_or(default_color);
    let width =
        i.o.w
            .filter(|w| *w != 0.0)
            .map_or(lw * weight(&i.k), |w| w * if small { 1.12 } else { 1.0 });
    if i.k == "tubeInk" {
        let mut path = Path::new(poly(&i.pts, false));
        path.stroke = color(p, "outline", "");
        path.width = i.o.w.unwrap_or(0.0);
        return vec![path];
    }
    let mut points: Vec<_> = i.o.prepend.iter().chain(&i.pts).cloned().collect();
    let mut heads = Vec::new();
    if i.o.head.is_some_and(|h| h != 0) {
        heads.push(arrow(&mut points, true, width));
    }
    if i.o.head == Some(2) || i.o.head_start {
        heads.push(arrow(&mut points, false, width));
    }
    let mut path = Path::new(poly(&points, false));
    path.stroke = stroke_color.clone();
    path.stroke_opacity = i.o.op.unwrap_or(opacity);
    path.width = width;
    path.dash = dash(&i.k);
    let mut paths = vec![path];
    for head in heads {
        let mut path = Path::new(head);
        path.fill = stroke_color.clone();
        path.fill_opacity = i.o.op.unwrap_or(opacity);
        paths.push(path);
    }
    paths
}

fn marker(i: &Item, p: &Palette, theme: &str, ow: f64, small: bool) -> Vec<Path> {
    let extra = if small { 0.3 } else { 0.0 };
    let paper = if theme == "dark" {
        "#eef0f3"
    } else {
        "#ffffff"
    };
    let outline = color(p, "outline", "");
    let (r, fill, stroke, width) = match i.k.as_str() {
        "pt" => (1.6 + extra, paper.into(), outline, ow * 1.1),
        "ptK" => (1.6 + extra, "currentColor".into(), "none".into(), 0.0),
        "ptL" => (2.5 + extra, paper.into(), outline, ow * 1.2),
        "cpt" => (2.2 + extra, color(p, "r", "left"), outline, ow),
        "spt" => (
            i.o.r.unwrap_or(1.35) + extra,
            color(p, "blue", ""),
            "none".into(),
            0.0,
        ),
        "dotK" => (i.o.r.unwrap_or(1.2), "#2b323c".into(), "none".into(), 0.0),
        "dotW" => (i.o.r.unwrap_or(1.0), "#ffffff".into(), "none".into(), 0.0),
        "dotI" => (
            i.o.r.unwrap_or(1.2),
            "currentColor".into(),
            "none".into(),
            0.0,
        ),
        _ => return Vec::new(),
    };
    let mut path = Path::new(circle(&i.pts[0], r));
    path.fill = fill;
    path.stroke = stroke;
    path.width = width;
    vec![path]
}
