//! Package preview PNGs: decoded off the UI thread, uploaded as egui textures, cached.
//!
//! These are small egui textures, not the wgpu tab thumbnails `display_budget` ledgers,
//! so the cache enforces its own entry and byte ceilings. Eviction leaves a `Failed`
//! slot instead of removing the entry, which keeps the same entry from being requested
//! again and again while a larger set than the ceiling is on screen.
use std::collections::{HashMap, VecDeque};
use std::io::Cursor;
use std::path::Path;

/// (component id, version).
pub(crate) type Key = (String, String);

const MAX_FILE_BYTES: u64 = 8 << 20;
/// Longest edge kept: the card and the detail well are both smaller.
const MAX_SIDE: usize = 512;
const MAX_ENTRIES: usize = 128;
/// Largest decoded image allowed before any pixel buffer is allocated.
const MAX_DECODED_BYTES: usize = 64 << 20;
/// Holds the most thumbnails one view asks for (see `MAX_THUMBNAILS`) at full size.
const MAX_BYTES: usize = 80 << 20;

pub(crate) struct Decoded {
    pub size: [usize; 2],
    pub rgba: Vec<u8>,
}

impl std::fmt::Debug for Decoded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Decoded({}x{})", self.size[0], self.size[1])
    }
}

/// Reads and decodes `path` into RGBA8, box-filtering anything above `MAX_SIDE`.
pub(crate) fn decode(path: &Path) -> Result<Decoded, String> {
    let length = std::fs::metadata(path)
        .map_err(|error| error.to_string())?
        .len();
    if length > MAX_FILE_BYTES {
        return Err(format!("preview is {length} bytes; limit {MAX_FILE_BYTES}"));
    }
    let bytes = std::fs::read(path).map_err(|error| error.to_string())?;
    let mut decoder = png::Decoder::new(Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().map_err(|error| error.to_string())?;
    // Declared size is checked before any buffer is sized from it.
    let info = reader.info();
    let declared = (info.width as usize)
        .checked_mul(info.height as usize)
        .and_then(|pixels| pixels.checked_mul(4));
    if declared.is_none_or(|bytes| bytes > MAX_DECODED_BYTES) {
        return Err(format!(
            "preview is {}x{} pixels; limit is {MAX_DECODED_BYTES} bytes decoded",
            info.width, info.height
        ));
    }
    let mut buffer = vec![0; reader.output_buffer_size().ok_or("preview is too large")?];
    let frame = reader
        .next_frame(&mut buffer)
        .map_err(|error| error.to_string())?;
    let (width, height) = (frame.width as usize, frame.height as usize);
    buffer.truncate(frame.buffer_size());
    let rgba = to_rgba(frame.color_type, buffer)?;
    if width == 0 || height == 0 || rgba.len() != width * height * 4 {
        return Err("preview has no pixels".into());
    }
    Ok(shrink(width, height, rgba))
}

fn to_rgba(color: png::ColorType, data: Vec<u8>) -> Result<Vec<u8>, String> {
    use png::ColorType::{Grayscale, GrayscaleAlpha, Rgb, Rgba};
    Ok(match color {
        Rgba => data,
        Rgb => data
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        Grayscale => data.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        GrayscaleAlpha => data
            .as_chunks::<2>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[0], p[0], p[1]])
            .collect(),
        png::ColorType::Indexed => return Err("indexed preview was not expanded".into()),
    })
}

fn shrink(width: usize, height: usize, rgba: Vec<u8>) -> Decoded {
    let factor = width.max(height).div_ceil(MAX_SIDE);
    if factor <= 1 {
        return Decoded {
            size: [width, height],
            rgba,
        };
    }
    let (w, h) = (width / factor, height / factor);
    let (w, h) = (w.max(1), h.max(1));
    let mut out = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        for x in 0..w {
            let mut sum = [0u32; 4];
            let mut count = 0u32;
            for dy in 0..factor {
                for dx in 0..factor {
                    let (sx, sy) = (x * factor + dx, y * factor + dy);
                    if sx < width && sy < height {
                        let at = (sy * width + sx) * 4;
                        for c in 0..4 {
                            sum[c] += u32::from(rgba[at + c]);
                        }
                        count += 1;
                    }
                }
            }
            out.extend(sum.map(|s| (s / count.max(1)) as u8));
        }
    }
    Decoded {
        size: [w, h],
        rgba: out,
    }
}

#[derive(Clone)]
enum Slot {
    Pending,
    Failed,
    Ready(egui::TextureHandle),
}

impl std::fmt::Debug for Slot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Pending => "Pending",
            Self::Failed => "Failed",
            Self::Ready(_) => "Ready",
        })
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Cache {
    slots: HashMap<Key, Slot>,
    order: VecDeque<Key>,
    bytes: usize,
}

impl Cache {
    pub(crate) fn texture(&self, key: &Key) -> Option<(egui::TextureId, [usize; 2])> {
        match self.slots.get(key) {
            Some(Slot::Ready(handle)) => Some((handle.id(), handle.size())),
            _ => None,
        }
    }

    pub(crate) fn absent(&self, key: &Key) -> bool {
        !self.slots.contains_key(key)
    }

    pub(crate) fn mark_pending(&mut self, key: Key) {
        self.slots.insert(key, Slot::Pending);
    }

    /// Returns an unsettled `Pending` slot to absent (its batch was abandoned).
    pub(crate) fn forget_pending(&mut self, key: &Key) {
        if matches!(self.slots.get(key), Some(Slot::Pending)) {
            self.slots.remove(key);
        }
    }

    pub(crate) fn fail(&mut self, key: Key) {
        self.slots.insert(key, Slot::Failed);
    }

    pub(crate) fn settle(
        &mut self,
        ctx: &egui::Context,
        key: Key,
        decoded: Result<Decoded, String>,
    ) {
        let Ok(image) = decoded else {
            return self.fail(key);
        };
        let Decoded { size, rgba } = image;
        let color = egui::ColorImage::from_rgba_unmultiplied(size, &rgba);
        drop(rgba);
        let name = format!("library-thumb:{}@{}", key.0, key.1);
        let handle = ctx.load_texture(name, color, egui::TextureOptions::LINEAR);
        // A key settled twice replaces its texture; count and order it once.
        if let Some(Slot::Ready(old)) = self.slots.get(&key) {
            let [w, h] = old.size();
            self.bytes = self.bytes.saturating_sub(w * h * 4);
            self.order.retain(|queued| *queued != key);
        }
        self.bytes += size[0] * size[1] * 4;
        self.order.push_back(key.clone());
        self.slots.insert(key, Slot::Ready(handle));
        self.evict();
    }

    fn evict(&mut self) {
        while self.order.len() > MAX_ENTRIES || self.bytes > MAX_BYTES {
            let Some(oldest) = self.order.pop_front() else {
                return;
            };
            // Evicted means absent, not failed: it can be decoded again when wanted.
            if let Some(Slot::Ready(handle)) = self.slots.remove(&oldest) {
                let [w, h] = handle.size();
                self.bytes = self.bytes.saturating_sub(w * h * 4);
            }
        }
    }

    pub(crate) fn clear(&mut self) {
        self.slots.clear();
        self.order.clear();
        self.bytes = 0;
    }

    #[cfg(test)]
    pub(crate) fn bytes(&self) -> usize {
        self.bytes
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.order.len()
    }
}
