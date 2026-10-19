//! The only module in this crate allowed to create `egui::Area`/`Window`, so floating
//! chrome shares one anchoring policy.

use crate::metrics::{Elevation, shadow};
use crate::theme::Theme;

/// Fixed-position, non-draggable floating area.
pub fn floating(id: egui::Id, pos: egui::Pos2, order: egui::Order) -> egui::Area {
    egui::Area::new(id)
        .order(order)
        .movable(false)
        .constrain(false)
        .fixed_pos(pos)
}

/// Floating surface: translucent fill, 1px border, rounded, elevated.
pub fn surface(t: &Theme, dark: bool, radius: u8, elevation: Elevation) -> egui::Frame {
    egui::Frame::new()
        .fill(t.floating)
        .stroke(egui::Stroke::new(1.0, t.border))
        .corner_radius(egui::CornerRadius::same(radius))
        .shadow(shadow(elevation, t, dark))
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    fn rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                rs_files(&p, out);
            } else if p.extension().is_some_and(|x| x == "rs") {
                out.push(p);
            }
        }
    }

    #[test]
    fn only_frame_creates_areas() {
        let mut files = Vec::new();
        rs_files(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
            &mut files,
        );
        assert!(!files.is_empty());
        let (a, w) = (["Area", "::new("].concat(), ["Window", "::new("].concat());
        for f in files {
            if f.file_name().is_some_and(|n| n == "frame.rs") {
                continue;
            }
            let text = std::fs::read_to_string(&f).unwrap_or_default();
            assert!(!text.contains(&a) && !text.contains(&w), "{}", f.display());
        }
    }
}
