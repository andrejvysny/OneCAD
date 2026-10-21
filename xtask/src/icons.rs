//! Native CAD icon baking from ordered, unprojected geometry.
#[path = "icons/geometry.rs"]
mod geometry;
#[path = "icons/paint.rs"]
mod paint;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::BTreeMap, error::Error, fs, path::Path};

const SIZES: [u32; 6] = [14, 16, 18, 20, 24, 32];
const GAP: &str = "#fe01fe";
type Point = Vec<f64>;

#[derive(Deserialize)]
struct Source {
    schema: u32,
    palette: BTreeMap<String, BTreeMap<String, Value>>,
    glyphs: BTreeMap<String, Glyph>,
}

#[derive(Clone, Deserialize)]
struct Glyph {
    fit: [f64; 4],
    items: Vec<Item>,
}

#[derive(Clone, Deserialize)]
struct Item {
    t: String,
    d3: bool,
    pts: Vec<Point>,
    #[serde(default)]
    holes: Vec<Vec<Point>>,
    #[serde(default)]
    fam: String,
    #[serde(default)]
    sh: String,
    #[serde(default)]
    k: String,
    #[serde(default)]
    w: f64,
    o: Options,
}

#[derive(Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Options {
    #[serde(default)]
    detail: bool,
    #[serde(default)]
    ns: bool,
    op: Option<f64>,
    so: Option<f64>,
    w: Option<f64>,
    r: Option<f64>,
    head: Option<u32>,
    #[serde(default)]
    head_start: bool,
    fixed: Option<String>,
    #[serde(default)]
    prepend: Vec<Point>,
}

#[derive(Serialize)]
struct Output {
    sizes: [u32; 6],
    gap: &'static str,
    icons: BTreeMap<String, BTreeMap<String, String>>,
}

pub fn run(args: &[String]) -> Result<(), Box<dyn Error>> {
    if args.iter().any(|a| a != "--check") {
        return Err("usage: cargo xtask bake-cad-icons [--check]".into());
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .ok_or("missing root")?;
    let source: Source =
        serde_json::from_slice(&fs::read(root.join("assets/native/cad-icons.json"))?)?;
    if source.schema != 1 {
        return Err("unsupported CAD icon geometry schema".into());
    }
    for theme in ["light", "dark"] {
        let bytes = bake(&source, theme)?;
        let path = root.join(format!("crates/onecad-widgets/assets/icons/{theme}.json"));
        if args.iter().any(|a| a == "--check") {
            if fs::read(&path)? != bytes {
                return Err(
                    format!("{} differs; run cargo xtask bake-cad-icons", path.display()).into(),
                );
            }
        } else {
            fs::create_dir_all(path.parent().ok_or("missing icon directory")?)?;
            fs::write(path, bytes)?;
        }
    }
    println!(
        "baked {} icons × {} sizes × 2 themes{}",
        source.glyphs.len() + 4,
        SIZES.len(),
        if args.is_empty() {
            ""
        } else {
            " (byte-exact check)"
        }
    );
    Ok(())
}

fn bake(source: &Source, theme: &str) -> Result<Vec<u8>, Box<dyn Error>> {
    let mut icons = BTreeMap::new();
    for name in source.glyphs.keys() {
        let mut sizes = BTreeMap::new();
        for size in SIZES {
            let compact = format!("{name}16");
            let glyph = if size <= 20 {
                source.glyphs.get(&compact)
            } else {
                None
            }
            .unwrap_or(&source.glyphs[name]);
            sizes.insert(
                size.to_string(),
                paint::render(glyph, &source.palette[theme], theme, size),
            );
        }
        icons.insert(name.clone(), sizes);
    }
    for (name, left, open) in [
        ("panelLeft", true, false),
        ("panelLeftOpen", true, true),
        ("panelRight", false, false),
        ("panelRightOpen", false, true),
    ] {
        if icons.contains_key(name) {
            return Err(format!("extra glyph {name} collides with the icon set").into());
        }
        icons.insert(
            name.into(),
            SIZES
                .into_iter()
                .map(|s| (s.to_string(), panel(left, open, s)))
                .collect(),
        );
    }
    let mut bytes = serde_json::to_vec(&Output {
        sizes: SIZES,
        gap: GAP,
        icons,
    })?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn panel(left: bool, open: bool, size: u32) -> String {
    let x = if left { "9.5" } else { "14.5" };
    let fill_x = if left { "4.2" } else { "15.5" };
    let fill = if open {
        format!("<rect x=\"{fill_x}\" y=\"5.7\" width=\"4.3\" height=\"12.6\" rx=\"1.3\" fill=\"currentColor\" fill-opacity=\".3\" stroke=\"none\"/>")
    } else {
        String::new()
    };
    format!("<svg width=\"{size}\" height=\"{size}\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.7\" stroke-linejoin=\"round\" stroke-linecap=\"round\"><rect x=\"3\" y=\"4.5\" width=\"18\" height=\"15\" rx=\"2.5\"/><path d=\"M{x} 4.5v15\"/>{fill}</svg>")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_generated_icon_matches_the_approved_registry_byte_for_byte() {
        let source: Source =
            serde_json::from_str(include_str!("../../assets/native/cad-icons.json")).unwrap();
        assert_eq!(source.glyphs.len(), 150);
        for (theme, expected) in [
            (
                "light",
                include_bytes!("../../crates/onecad-widgets/assets/icons/light.json").as_slice(),
            ),
            (
                "dark",
                include_bytes!("../../crates/onecad-widgets/assets/icons/dark.json").as_slice(),
            ),
        ] {
            assert_eq!(bake(&source, theme).unwrap(), expected);
        }
    }

    #[test]
    fn geometry_changes_are_not_hidden_by_a_baked_output_cache() {
        let mut source: Source =
            serde_json::from_str(include_str!("../../assets/native/cad-icons.json")).unwrap();
        let approved = bake(&source, "light").unwrap();
        source.glyphs.get_mut("select").unwrap().items[0].pts[0][0] += 1.0;
        assert_ne!(bake(&source, "light").unwrap(), approved);
    }

    #[test]
    fn unknown_options_fail_without_touching_outputs() {
        assert!(run(&["--unknown".into()]).is_err());
    }
}
