//! Static mock catalogue behind the extensions facade; nothing here is fetched or installed.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trust {
    Verified,
    Community,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
    Workspaces,
    Tools,
    Generators,
    ImportExport,
    Materials,
    LibraryPacks,
}

/// Chip labels in display order; index 0 is "All" and has no category.
pub const CATEGORY_CHIPS: [(&str, Option<Category>); 7] = [
    ("All", None),
    ("Workspaces", Some(Category::Workspaces)),
    ("Tools", Some(Category::Tools)),
    ("Generators", Some(Category::Generators)),
    ("Import & export", Some(Category::ImportExport)),
    ("Materials", Some(Category::Materials)),
    ("Library packs", Some(Category::LibraryPacks)),
];

pub const SOURCES: [&str; 4] = [
    "All sources",
    "OneCAD catalogue",
    "Workshop registry",
    "Developer",
];

#[derive(Clone, Copy, Debug)]
pub struct Permission {
    pub icon: &'static str,
    pub title: &'static str,
    pub reason: &'static str,
}

#[derive(Clone, Copy, Debug)]
pub struct Adds {
    pub icon: &'static str,
    pub name: &'static str,
    pub desc: &'static str,
    pub location: &'static str,
}

#[derive(Clone, Copy, Debug)]
pub struct Release {
    pub version: &'static str,
    pub date: &'static str,
    pub notes: &'static [&'static str],
}

#[derive(Clone, Copy, Debug)]
pub struct Ext {
    pub id: &'static str,
    pub name: &'static str,
    pub publisher: &'static str,
    pub trust: Trust,
    pub icon: &'static str,
    pub desc: &'static str,
    pub long_desc: &'static str,
    pub meta: &'static str,
    pub category: Category,
    pub version: &'static str,
    pub size: &'static str,
    pub updated: &'static str,
    pub permissions: &'static [Permission],
    pub no_access: &'static str,
    pub adds: &'static [Adds],
    pub log: &'static [Release],
}

const READ: Permission = Permission {
    icon: "eyeOn",
    title: "Read document",
    reason: "Find inside corners and pockets",
};
const MODIFY: Permission = Permission {
    icon: "edit",
    title: "Modify document",
    reason: "Add relief cuts through the modeling service",
};

const CNC_LOG: &[Release] = &[
    Release {
        version: "1.4.0",
        date: "Sep 28, 2026",
        notes: &[
            "Tool reach layer shows unreachable areas in red",
            "Dogbone relief respects the default tool diameter",
            "Stock setup remembers the last used material",
        ],
    },
    Release {
        version: "1.3.2",
        date: "Aug 14, 2026",
        notes: &[
            "Fixes T-bone orientation on rotated pockets",
            "Machinability section no longer flickers while editing",
        ],
    },
    Release {
        version: "1.3.0",
        date: "Jul 02, 2026",
        notes: &[
            "Adds the Machinability inspector section",
            "Minimal relief style for thin walls",
            "Setups can be reordered in the Explorer",
        ],
    },
];

const FIRST_RELEASE: &[Release] = &[Release {
    version: "Initial release",
    date: "2026",
    notes: &["First public version of this extension"],
}];

const CNC_ADDS: &[Adds] = &[
    Adds {
        icon: "panelLeft",
        name: "CNC",
        desc: "Workspace with Stock and Setups in the Explorer",
        location: "Workspace menu",
    },
    Adds {
        icon: "origin",
        name: "Dogbone, T-bone, Stock…",
        desc: "4 tools",
        location: "Pill · edge, body",
    },
    Adds {
        icon: "layers",
        name: "Tool reach",
        desc: "Areas the current tool can't reach, in red",
        location: "Layers menu",
    },
    Adds {
        icon: "menu",
        name: "Machinability",
        desc: "Corner radius vs. tool for a picked edge",
        location: "Inspector section",
    },
];

const PRINT_ADDS: &[Adds] = &[
    Adds {
        icon: "export",
        name: "3MF export",
        desc: "Print settings and mirrored bodies",
        location: "File → Export",
    },
    Adds {
        icon: "layers",
        name: "Overhang map",
        desc: "Overhangs per body material",
        location: "Layers menu",
    },
];

const KICAD_ADDS: &[Adds] = &[
    Adds {
        icon: "import",
        name: ".kicad_pcb import",
        desc: "Import KiCad boards",
        location: "File → Import",
    },
    Adds {
        icon: "component",
        name: "Enclosure, Standoffs",
        desc: "Generate a fitted enclosure",
        location: "Pill · body",
    },
];

const JOINERY_ADDS: &[Adds] = &[Adds {
    icon: "component",
    name: "Finger, Dovetail, Mortise",
    desc: "3 generators",
    location: "Pill · body",
}];

const PLASTICS_ADDS: &[Adds] = &[Adds {
    icon: "material",
    name: "42 materials",
    desc: "Density, strength and print temperatures",
    location: "Material picker, Inspector",
}];

const IGES_ADDS: &[Adds] = &[Adds {
    icon: "export",
    name: ".iges import and export",
    desc: "IGES 5.3 surfaces and wireframes",
    location: "File → Import",
}];

const FILES: Permission = Permission {
    icon: "folder",
    title: "Files on disk",
    reason: "Read the .kicad_pcb and its 3D models",
};

pub const CATALOG: [Ext; 6] = [
    Ext {
        id: "cnc-prep",
        name: "CNC Prep",
        publisher: "Millhouse",
        trust: Trust::Verified,
        icon: "panelLeft",
        desc: "Dogbones, tool-reach checks and stock setup for 3-axis milling.",
        long_desc: "Find inside corners a round cutter can't reach and add dogbone or T-bone reliefs in one click. Set up stock and see unreachable areas while you model. Reliefs are added as regular Cut features in History.",
        meta: "1 workspace · 4 tools · 1 layer",
        category: Category::Workspaces,
        version: "1.4.0",
        size: "3.1 MB",
        updated: "Sep 28, 2026",
        permissions: &[READ, MODIFY],
        no_access: "No files, network, clipboard or native code.",
        adds: CNC_ADDS,
        log: CNC_LOG,
    },
    Ext {
        id: "print-prep",
        name: "Print Prep",
        publisher: "Layerline",
        trust: Trust::Verified,
        icon: "layers",
        desc: "Plate layout, overhang map and 3MF export with print settings.",
        long_desc: "Lay bodies out on a build plate, see which overhangs need support and export a 3MF with your print settings attached.",
        meta: "1 format · 1 layer · 1 section",
        category: Category::Tools,
        version: "1.2.0",
        size: "2.4 MB",
        updated: "Aug 30, 2026",
        permissions: &[
            Permission {
                icon: "eyeOn",
                title: "Read document",
                reason: "Measure overhangs and plate fit",
            },
            Permission {
                icon: "folder",
                title: "Files on disk",
                reason: "Write 3MF files you choose",
            },
        ],
        no_access: "No network, clipboard or native code.",
        adds: PRINT_ADDS,
        log: FIRST_RELEASE,
    },
    Ext {
        id: "kicad-bridge",
        name: "KiCad Bridge",
        publisher: "tinkerboard",
        trust: Trust::Community,
        icon: "import",
        desc: "Import KiCad boards and generate a fitted enclosure.",
        long_desc: "Bring a KiCad board into OneCAD, keep its outline and holes, and generate an enclosure with standoffs around it.",
        meta: "1 format · 2 tools",
        category: Category::ImportExport,
        version: "0.6.0",
        size: "920 KB",
        updated: "Sep 02, 2026",
        permissions: &[
            Permission {
                icon: "edit",
                title: "Modify document",
                reason: "Create the board body and enclosure",
            },
            FILES,
        ],
        no_access: "No network, clipboard or native code.",
        adds: KICAD_ADDS,
        log: FIRST_RELEASE,
    },
    Ext {
        id: "wood-joinery",
        name: "Wood Joinery",
        publisher: "Grainworks",
        trust: Trust::Community,
        icon: "component",
        desc: "Finger, dovetail and mortise joints between touching bodies.",
        long_desc: "Pick two touching bodies and generate matching finger, dovetail or mortise joints with clearances for your saw kerf.",
        meta: "3 generators",
        category: Category::Generators,
        version: "0.4.0",
        size: "1.1 MB",
        updated: "Sep 10, 2026",
        permissions: &[Permission {
            icon: "edit",
            title: "Modify document",
            reason: "Cut joints into the picked bodies",
        }],
        no_access: "No files, network, clipboard or native code.",
        adds: JOINERY_ADDS,
        log: FIRST_RELEASE,
    },
    Ext {
        id: "engineering-plastics",
        name: "Engineering Plastics",
        publisher: "Polymatter",
        trust: Trust::Verified,
        icon: "material",
        desc: "42 materials with density, strength and print temperatures.",
        long_desc: "A curated material set for printed and machined plastics, with density, strength and recommended print temperatures.",
        meta: "42 materials",
        category: Category::Materials,
        version: "2.0.1",
        size: "1.8 MB",
        updated: "Sep 19, 2026",
        permissions: &[],
        no_access: "No document, file or network access.",
        adds: PLASTICS_ADDS,
        log: FIRST_RELEASE,
    },
    Ext {
        id: "iges-interchange",
        name: "IGES Interchange",
        publisher: "Fieldline",
        trust: Trust::Verified,
        icon: "export",
        desc: "Read and write IGES 5.3 surfaces and wireframes.",
        long_desc: "Exchange surfaces and wireframes with systems that still speak IGES 5.3.",
        meta: "1 format",
        category: Category::ImportExport,
        version: "1.0.3",
        size: "1.3 MB",
        updated: "Jun 21, 2026",
        permissions: &[Permission {
            icon: "folder",
            title: "Files on disk",
            reason: "Read and write the IGES files you pick",
        }],
        no_access: "No network, clipboard or native code.",
        adds: IGES_ADDS,
        log: FIRST_RELEASE,
    },
];

pub fn find(id: &str) -> Option<&'static Ext> {
    CATALOG.iter().find(|e| e.id == id)
}

/// Mock registry the extension is listed in, for the source filter.
pub fn source_of(e: &Ext) -> usize {
    match e.trust {
        Trust::Verified => 1,
        Trust::Community => 2,
    }
}

/// Pending update shown by the 21a review sheet.
pub struct UpdateInfo {
    pub from: &'static str,
    pub to: &'static str,
    pub notes: [&'static str; 3],
    pub permission: Permission,
    pub unchanged: &'static str,
}

pub const PRINT_UPDATE: UpdateInfo = UpdateInfo {
    from: "1.2.0",
    to: "1.3.0",
    notes: [
        "Send plates straight to networked printers",
        "Overhang map respects per-body material",
        "Fixes 3MF export of mirrored bodies",
    ],
    permission: Permission {
        icon: "import",
        title: "Network",
        reason: "Send print jobs to printers on your local network",
    },
    unchanged: "Unchanged: Read document, Files on disk.",
};

pub fn update_info(id: &str) -> Option<&'static UpdateInfo> {
    (id == "print-prep").then_some(&PRINT_UPDATE)
}
