//! Mock ISO 4762 spec table and the pure functions deriving part text from a selection (10a).

pub struct SizeRow {
    pub size: &'static str,
    pub d: f32,
    pub pitch: &'static str,
    pub dk: f32,
    pub k: f32,
    pub s: &'static str,
    pub lo: u32,
    pub hi: u32,
    pub clearance: f32,
    pub cb: (f32, f32),
    pub tap: f32,
}

macro_rules! row {
    ($size:literal, $d:literal, $pitch:literal, $dk:literal, $k:literal, $s:literal, $lo:literal, $hi:literal, $clr:literal, ($cd:literal, $ch:literal), $tap:literal) => {
        SizeRow {
            size: $size,
            d: $d,
            pitch: $pitch,
            dk: $dk,
            k: $k,
            s: $s,
            lo: $lo,
            hi: $hi,
            clearance: $clr,
            cb: ($cd, $ch),
            tap: $tap,
        }
    };
}

pub const SIZES: [SizeRow; 8] = [
    row!(
        "M2",
        2.0,
        "0.4",
        3.8,
        2.0,
        "1.5",
        3,
        20,
        2.4,
        (4.5, 2.6),
        1.6
    ),
    row!(
        "M2.5",
        2.5,
        "0.45",
        4.5,
        2.5,
        "2",
        4,
        25,
        2.9,
        (5.5, 3.1),
        2.05
    ),
    row!(
        "M3",
        3.0,
        "0.5",
        5.5,
        3.0,
        "2.5",
        4,
        35,
        3.4,
        (6.0, 3.4),
        2.5
    ),
    row!("M4", 4.0, "0.7", 7.0, 4.0, "3", 5, 45, 4.5, (8.0, 4.4), 3.3),
    row!(
        "M5",
        5.0,
        "0.8",
        8.5,
        5.0,
        "4",
        6,
        50,
        5.5,
        (10.0, 5.4),
        4.2
    ),
    row!("M6", 6.0, "1", 10.0, 6.0, "5", 8, 60, 6.6, (11.0, 6.5), 5.0),
    row!(
        "M8",
        8.0,
        "1.25",
        13.0,
        8.0,
        "6",
        10,
        80,
        9.0,
        (15.0, 8.6),
        6.8
    ),
    row!(
        "M10",
        10.0,
        "1.5",
        16.0,
        10.0,
        "8",
        12,
        100,
        11.0,
        (18.0, 10.8),
        8.5
    ),
];

pub const LENGTHS: [u32; 19] = [
    3, 4, 5, 6, 8, 10, 12, 16, 20, 25, 30, 35, 40, 45, 50, 60, 70, 80, 100,
];

/// Material/class labels and densities in g/cm3.
pub const MATERIALS: [(&str, f32); 3] = [
    ("Steel 12.9", 7.85),
    ("A2 stainless", 7.9),
    ("A4 stainless", 8.0),
];

pub struct Family {
    pub group: &'static str,
    pub name: &'static str,
    pub standard: &'static str,
    pub two_line: bool,
}

const fn fam(
    group: &'static str,
    name: &'static str,
    standard: &'static str,
    two_line: bool,
) -> Family {
    Family {
        group,
        name,
        standard,
        two_line,
    }
}

pub const FAMILIES: [Family; 10] = [
    fam("SCREWS & BOLTS", "Socket head cap screw", "ISO 4762", true),
    fam("SCREWS & BOLTS", "Button head", "ISO 7380", true),
    fam("SCREWS & BOLTS", "Countersunk", "ISO 10642", true),
    fam("SCREWS & BOLTS", "Hex head bolt", "ISO 4017", true),
    fam("SCREWS & BOLTS", "Set screw, cup point", "ISO 4029", true),
    fam("NUTS & WASHERS", "Hex nut", "ISO 4032", false),
    fam("NUTS & WASHERS", "Nyloc nut", "ISO 10511", false),
    fam("NUTS & WASHERS", "Plain washer", "ISO 7089", false),
    fam("BEARINGS", "Deep groove", "ISO 15", false),
    fam("BEARINGS", "Flanged", "ISO 15", false),
];

pub fn subtitle(family: usize) -> String {
    let f = &FAMILIES[family];
    let standards = if family == 0 {
        "ISO 4762 / DIN 912".to_owned()
    } else {
        f.standard.to_owned()
    };
    format!("{standards} · pick a size, then a length")
}

pub fn lengths_for(size: usize) -> Vec<u32> {
    let row = &SIZES[size];
    LENGTHS
        .into_iter()
        .filter(|l| (row.lo..=row.hi).contains(l))
        .collect()
}

/// Nearest offered length, so changing size never leaves an impossible selection.
pub fn clamp_length(size: usize, length: u32) -> u32 {
    lengths_for(size)
        .into_iter()
        .min_by_key(|l| l.abs_diff(length))
        .unwrap_or(length)
}

pub fn part_code(family: usize, size: usize, length: u32) -> String {
    format!(
        "{} {}×{}",
        FAMILIES[family].standard, SIZES[size].size, length
    )
}

pub fn mass_g(size: usize, length: u32, material: usize) -> f32 {
    let r = &SIZES[size];
    let shank = std::f32::consts::FRAC_PI_4 * r.d * r.d * length as f32;
    let head = std::f32::consts::FRAC_PI_4 * r.dk * r.dk * r.k;
    // 0.79 accounts for the hex socket and thread relief in the mock volume.
    (shank + head) * 0.79 * MATERIALS[material].1 * 1e-3
}

pub fn part_summary(size: usize, length: u32, material: usize, modeled: bool) -> String {
    let thread = if modeled {
        "modeled thread"
    } else {
        "cosmetic thread"
    };
    let mass = mass_g(size, length, material);
    format!("{} · {thread} · {mass:.1} g", MATERIALS[material].0)
}

pub fn derived(size: usize) -> [(&'static str, String); 3] {
    let r = &SIZES[size];
    [
        ("Clearance hole", format!("Ø {} mm", r.clearance)),
        ("Counterbore", format!("Ø {} × {} mm", r.cb.0, r.cb.1)),
        ("Tap drill", format!("Ø {} mm", r.tap)),
    ]
}

/// Selection state of the standard-part page.
#[derive(Clone, Debug)]
pub struct StdState {
    pub family: usize,
    pub size: usize,
    pub length: u32,
    pub material: usize,
    pub thread: usize,
    pub insert_as: usize,
    pub added: bool,
    pub inserted: bool,
}

impl Default for StdState {
    fn default() -> Self {
        Self {
            family: 0,
            size: 3,
            length: 12,
            material: 1,
            thread: 0,
            insert_as: 0,
            added: false,
            inserted: false,
        }
    }
}

impl StdState {
    pub fn select_size(&mut self, size: usize) {
        self.size = size;
        self.length = clamp_length(size, self.length);
        self.inserted = false;
    }
}
