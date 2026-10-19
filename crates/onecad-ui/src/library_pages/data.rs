//! Static mock data for the library facade (Final 08a-10a). No library service backs it.
use super::kit::ChipKind;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Mine,
    Project,
    Standard,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Parametric,
    Static,
}

pub struct Component {
    pub name: &'static str,
    pub meta: &'static str,
    pub icon: &'static str,
    pub source: Source,
    pub kind: Kind,
    pub update: bool,
    pub chips: &'static [(&'static str, ChipKind)],
}

const FX7: (&str, ChipKind) = ("fx 7", ChipKind::Accent);
const STD: (&str, ChipKind) = ("Standard", ChipKind::Plain);
const STATIC: (&str, ChipKind) = ("Static", ChipKind::Plain);

const fn comp(
    name: &'static str,
    meta: &'static str,
    icon: &'static str,
    source: Source,
    kind: Kind,
    chips: &'static [(&'static str, ChipKind)],
) -> Component {
    Component {
        name,
        meta,
        icon,
        source,
        kind,
        update: false,
        chips,
    }
}

use Kind::{Parametric as P, Static as S};
use Source::{Mine, Project, Standard};

pub const COMPONENTS: [Component; 14] = [
    comp(
        "Mounting bracket",
        "v3 · My components · 3 models",
        "body",
        Mine,
        P,
        &[FX7, ("3 presets", ChipKind::Plain)],
    ),
    comp(
        "Socket head cap screw",
        "ISO 4762 · M2–M12",
        "thread",
        Standard,
        P,
        &[STD, ("84 sizes", ChipKind::Plain)],
    ),
    comp(
        "Deep groove ball bearing",
        "ISO 15 · 608 · 8×22×7",
        "revolve",
        Standard,
        P,
        &[STD],
    ),
    comp(
        "Pi 5 standoff set",
        "This project · 4 bodies",
        "hole",
        Project,
        S,
        &[STATIC],
    ),
    Component {
        update: true,
        ..comp(
            "NEMA 17 motor mount",
            "v2 · edited Oct 14",
            "body",
            Mine,
            P,
            &[("fx 5", ChipKind::Accent)],
        )
    },
    comp(
        "Print-in-place hinge",
        "v1 · edited Oct 2",
        "mirror",
        Mine,
        P,
        &[("fx 4", ChipKind::Accent)],
    ),
    comp(
        "Snap-fit enclosure lid",
        "v5 · edited Sep 28",
        "shell",
        Mine,
        P,
        &[("fx 6", ChipKind::Accent), ("2 presets", ChipKind::Plain)],
    ),
    comp(
        "Cable clip 6 mm",
        "v1 · edited Sep 20",
        "sweep",
        Mine,
        S,
        &[STATIC],
    ),
    comp(
        "Base plate 120",
        "This project · 1 body",
        "extrude",
        Project,
        P,
        &[("fx 3", ChipKind::Accent)],
    ),
    comp(
        "Fan duct adapter",
        "This project · 2 bodies",
        "shell",
        Project,
        S,
        &[STATIC],
    ),
    comp(
        "Sensor mount",
        "This project · 1 body",
        "body",
        Project,
        P,
        &[("fx 2", ChipKind::Accent)],
    ),
    comp(
        "Cable guide",
        "This project · 1 body",
        "sweep",
        Project,
        S,
        &[STATIC],
    ),
    comp(
        "Hex nut",
        "ISO 4032 · M2–M10",
        "polygon",
        Standard,
        P,
        &[STD, ("9 sizes", ChipKind::Plain)],
    ),
    comp(
        "Plain washer",
        "ISO 7089 · M2–M10",
        "revolve",
        Standard,
        P,
        &[STD, ("9 sizes", ChipKind::Plain)],
    ),
];

/// "Recently used" cards, in display order (indexes into [`COMPONENTS`]).
pub const RECENT: [usize; 4] = [0, 1, 2, 3];

pub struct Category {
    pub name: &'static str,
    pub types: u32,
    pub icon: &'static str,
}

pub const CATEGORIES: [Category; 5] = [
    Category {
        name: "Screws & bolts",
        types: 46,
        icon: "thread",
    },
    Category {
        name: "Nuts & washers",
        types: 22,
        icon: "polygon",
    },
    Category {
        name: "Bearings",
        types: 18,
        icon: "revolve",
    },
    Category {
        name: "Standoffs & spacers",
        types: 13,
        icon: "hole",
    },
    Category {
        name: "Pins & dowels",
        types: 10,
        icon: "extrude",
    },
];

/// Source filter indexes: 0 All, 1 My components, 2 This project, 3 Standard parts.
/// Type filter indexes: 0 All types, 1 Parametric, 2 Static.
pub fn kind_matches(c: &Component, kind: usize) -> bool {
    match kind {
        1 => c.kind == Kind::Parametric,
        2 => c.kind == Kind::Static,
        _ => true,
    }
}

pub fn text_matches(c: &Component, query: &str) -> bool {
    let q = query.trim().to_lowercase();
    q.is_empty() || c.name.to_lowercase().contains(&q) || c.meta.to_lowercase().contains(&q)
}

/// Grid content for the current filters; pure so the tabs are unit-testable.
#[derive(Debug, PartialEq, Eq)]
pub struct Sections {
    pub recent: Vec<usize>,
    pub tiles: bool,
    pub own_title: &'static str,
    pub own: Vec<usize>,
}

pub fn sections(source: usize, kind: usize, query: &str) -> Sections {
    let keep = |i: &usize| {
        let c = &COMPONENTS[*i];
        kind_matches(c, kind) && text_matches(c, query)
    };
    let all = || 0..COMPONENTS.len();
    let by = |s: Source| {
        all()
            .filter(move |i| COMPONENTS[*i].source == s)
            .filter(keep)
    };
    let tiles = matches!(source, 0 | 3)
        && kind != 2
        && CATEGORIES.iter().any(|c| {
            query.trim().is_empty() || c.name.to_lowercase().contains(&query.to_lowercase())
        });
    match source {
        1 => Sections {
            recent: Vec::new(),
            tiles: false,
            own_title: "My components",
            own: by(Mine).collect(),
        },
        2 => Sections {
            recent: Vec::new(),
            tiles: false,
            own_title: "This project",
            own: by(Project).collect(),
        },
        3 => Sections {
            recent: Vec::new(),
            tiles,
            own_title: "Popular parts",
            own: by(Standard).collect(),
        },
        _ => Sections {
            recent: RECENT.iter().copied().filter(keep).collect(),
            tiles,
            own_title: "My components",
            own: by(Mine).filter(|i| !RECENT.contains(i)).collect(),
        },
    }
}

/// Cards per row for a minimum card width and gap.
pub fn columns(avail: f32, min: f32, gap: f32) -> usize {
    (((avail + gap) / (min + gap)).floor() as usize).max(1)
}

// ---- 09a component detail ----

pub struct Param {
    pub name: &'static str,
    pub kind: &'static str,
    pub default: &'static str,
    pub range: &'static str,
}

pub const PARAMS: [Param; 7] = [
    Param {
        name: "width",
        kind: "Length",
        default: "60 mm",
        range: "40 – 120 mm, step 5",
    },
    Param {
        name: "height",
        kind: "Length",
        default: "40 mm",
        range: "30 – 80 mm",
    },
    Param {
        name: "thickness",
        kind: "Length",
        default: "4 mm",
        range: "2 – 8 mm",
    },
    Param {
        name: "hole_size",
        kind: "Option",
        default: "M4",
        range: "M3 · M4 · M5",
    },
    Param {
        name: "fillets",
        kind: "Toggle",
        default: "On",
        range: "Suppresses Fillet 1 when off",
    },
    Param {
        name: "fillet_r",
        kind: "Expression",
        default: "2 mm",
        range: "= thickness / 2",
    },
    Param {
        name: "material",
        kind: "Material",
        default: "Al 6061",
        range: "Any",
    },
];

/// Preset name and width x height x thickness in mm.
pub const PRESETS: [(&str, f32, f32, f32); 3] = [
    ("Small", 40.0, 30.0, 3.0),
    ("Standard", 60.0, 40.0, 4.0),
    ("Heavy", 90.0, 60.0, 6.0),
];

pub fn dims_label(preset: usize) -> String {
    let (_, w, h, t) = PRESETS[preset.min(2)];
    format!("{w} × {h} × {t}")
}

pub const VERSIONS: [(&str, &str); 3] = [
    ("v3", "Oct 17 · Fillet radius follows plate thickness"),
    ("v2", "Sep 30 · Added Small / Heavy presets"),
    ("v1", "Sep 12 · Initial L-bracket"),
];

pub const USED_IN: [(&str, &str); 3] = [
    ("Pi 5 case.onecad", "2 instances · Oct 17"),
    ("Workshop shelf.onecad", "4 instances · Oct 9"),
    ("Drone frame.onecad", "1 instance · Sep 30"),
];

pub const MATERIALS: [&str; 3] = ["Aluminium 6061", "Steel S235", "PETG"];
/// g/cm3 for [`MATERIALS`].
pub const DENSITY: [f32; 3] = [2.7, 7.85, 1.27];
pub const HOLES: [&str; 3] = ["M3", "M4", "M5"];
