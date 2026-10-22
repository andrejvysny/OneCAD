//! Golden routed-event text for every `fixtures/ios/traces/*.json`. Each
//! `expected/<name>.txt` starts with the FNV-1a digest of the lines below it.
//! Regenerate only on purpose: `ONECAD_UPDATE_TOUCH_TRACES=1 cargo test -p
//! onecad-interaction --test touch_traces`, then review the diff.
use onecad_interaction::touch::trace;
use std::path::{Path, PathBuf};

const REQUIRED: &[&str] = &[
    "pencil_line_stroke",
    "pencil_arc_stroke",
    "finger_tap_on_control",
    "finger_orbit",
    "pinch_with_cancel",
    "pencil_finger_two_handed",
    "coalesced_predicted_estimation",
    "view_generation_change",
    "cancel_all_mid_stroke",
    "missing_optional_capabilities",
];

fn traces_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/ios/traces")
}

/// Only an explicit `=1` rewrites goldens; `=0` or an empty value checks them.
fn updating() -> bool {
    std::env::var("ONECAD_UPDATE_TOUCH_TRACES").is_ok_and(|value| value == "1")
}

fn expected_text(lines: &[String]) -> String {
    format!("digest {}\n{}\n", trace::digest(lines), lines.join("\n"))
}

#[test]
fn every_trace_matches_its_golden_routed_events() {
    let update = updating();
    let dir = traces_dir();
    if update {
        // Git does not keep an empty `expected/` on a fresh checkout.
        std::fs::create_dir_all(dir.join("expected")).expect("expected directory");
    }
    let mut names = Vec::new();
    for entry in std::fs::read_dir(&dir).expect("traces directory") {
        let path = entry.expect("entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let text = std::fs::read_to_string(&path).expect("trace text");
        let parsed = trace::parse(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let stem = path.file_stem().and_then(|s| s.to_str()).expect("stem");
        assert_eq!(parsed.name, stem, "trace name must match its file");
        let actual = expected_text(&trace::run(&parsed));
        let golden = dir.join("expected").join(format!("{stem}.txt"));
        if update {
            std::fs::write(&golden, &actual).expect("write golden");
        } else {
            let expected = std::fs::read_to_string(&golden)
                .unwrap_or_else(|_| panic!("missing golden {}", golden.display()));
            assert_eq!(actual, expected, "{stem}: routed events changed");
        }
        names.push(stem.to_owned());
    }
    for required in REQUIRED {
        assert!(
            names.iter().any(|n| n == required),
            "missing trace {required}"
        );
    }
}

#[test]
fn the_digest_line_covers_the_event_lines() {
    if updating() {
        // The other test is rewriting these files concurrently.
        return;
    }
    let dir = traces_dir().join("expected");
    for required in REQUIRED {
        let text = std::fs::read_to_string(dir.join(format!("{required}.txt"))).expect("golden");
        let (head, body) = text.split_once('\n').expect("digest line");
        let lines: Vec<String> = body.lines().map(str::to_owned).collect();
        assert_eq!(
            head,
            format!("digest {}", trace::digest(&lines)),
            "{required}"
        );
    }
}
