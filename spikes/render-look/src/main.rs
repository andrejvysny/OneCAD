//! ONECAD-259 (P0.4) render-look spike: headless wgpu renders of the MESH1 fixtures in today's
//! look (baseline) and a candidate quality look, plus per-pass GPU timings.
//!
//! `cargo run --release` writes spikes/render-look/out/*.png and prints the measurements.

mod bench;
mod camera;
mod gpu;
mod ibl;
mod look;
mod mesh1;
mod render;
mod scene;
mod timing;
mod tokens;

use std::path::{Path, PathBuf};
use std::time::Instant;

use camera::Camera;
use gpu::Gpu;
use look::Look;
use render::{Parts, Renderer};
use scene::Scene;
use tokens::{Palette, Theme};

const SIZE: [u32; 2] = [2560, 1600];

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

struct Image {
    w: u32,
    h: u32,
    rgba: Vec<u8>,
}

fn readback(gpu: &Gpu, tex: &wgpu::Texture, size: [u32; 2]) -> Image {
    let bpr = size[0] * 4;
    let buf = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: (bpr * size[1]) as u64,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut enc = gpu.device.create_command_encoder(&Default::default());
    enc.copy_texture_to_buffer(
        tex.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buf,
            layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(bpr), rows_per_image: Some(size[1]) },
        },
        wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
    );
    gpu.queue.submit([enc.finish()]);
    let slice = buf.slice(..);
    slice.map_async(wgpu::MapMode::Read, |_| {});
    gpu.wait();
    let rgba = slice.get_mapped_range().map(|v| v.to_vec()).unwrap_or_default();
    Image { w: size[0], h: size[1], rgba }
}

fn write_png(path: &Path, img: &Image) -> Result<(), String> {
    // Every output is an opaque frame; a partly transparent pixel means a pipeline wrote alpha.
    let clear = img.rgba.chunks_exact(4).filter(|p| p[3] != 255).count();
    if clear > 0 {
        return Err(format!("{}: {clear} pixels with alpha below 255", path.display()));
    }
    let file = std::fs::File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), img.w, img.h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
    let mut w = enc.write_header().map_err(|e| e.to_string())?;
    w.write_image_data(&img.rgba).map_err(|e| e.to_string())?;
    println!("  wrote {}", path.strip_prefix(repo()).unwrap_or(path).display());
    Ok(())
}

/// 2×2 box downsample (sRGB bytes averaged in linear light).
fn half(img: &Image) -> Image {
    let (w, h) = (img.w / 2, img.h / 2);
    let lin = |b: u8| {
        let c = b as f32 / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    let enc = |l: f32| {
        let c = if l <= 0.0031308 { l * 12.92 } else { 1.055 * l.powf(1.0 / 2.4) - 0.055 };
        (c * 255.0).round().clamp(0.0, 255.0) as u8
    };
    let mut rgba = vec![0u8; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            for c in 0..4 {
                let at = |dx: u32, dy: u32| img.rgba[(((2 * y + dy) * img.w + 2 * x + dx) * 4 + c) as usize];
                let o = ((y * w + x) * 4 + c) as usize;
                rgba[o] = if c == 3 { 255 } else { enc((lin(at(0, 0)) + lin(at(1, 0)) + lin(at(0, 1)) + lin(at(1, 1))) * 0.25) };
            }
        }
    }
    Image { w, h, rgba }
}

fn side_by_side(a: &Image, b: &Image) -> Image {
    let (w, h) = (a.w + b.w, a.h.max(b.h));
    let mut rgba = vec![255u8; (w * h * 4) as usize];
    for (img, x0) in [(a, 0), (b, a.w)] {
        for y in 0..img.h {
            let src = &img.rgba[(y * img.w * 4) as usize..((y + 1) * img.w * 4) as usize];
            let o = ((y * w + x0) * 4) as usize;
            rgba[o..o + src.len()].copy_from_slice(src);
        }
    }
    Image { w, h, rgba }
}

fn zoom(img: &Image, k: u32) -> Image {
    let (w, h) = (img.w * k, img.h * k);
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            let s = (((y / k) * img.w + x / k) * 4) as usize;
            rgba.extend_from_slice(&img.rgba[s..s + 4]);
        }
    }
    Image { w, h, rgba }
}

fn qual_clone(l: &Look) -> Look {
    Look { quality: l.quality, theme: l.theme, palette: l.palette.clone(), sil: l.sil }
}

fn crop(img: &Image, x0: u32, y0: u32, w: u32, h: u32) -> Image {
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    for y in y0..y0 + h {
        let s = ((y * img.w + x0) * 4) as usize;
        rgba.extend_from_slice(&img.rgba[s..s + (w * 4) as usize]);
    }
    Image { w, h, rgba }
}

/// How feature edges and silhouettes reach the MSAA target.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Lines {
    /// Analytic box-filtered coverage, alpha-blended (the default quality look).
    Blend,
    /// The same analytic coverage turned into the sample mask by alpha-to-coverage.
    A2c,
    /// Hard coverage (the quad is the line, round caps cut at the pixel centre) through
    /// alpha-to-coverage: MSAA-only edges, as today's LineMaterial draws them.
    Msaa,
}

fn render_once(gpu: &Gpu, r: &Renderer, scene: &Scene, cam: &Camera, look: &Look, lines: Lines) -> Image {
    let mut u = look::frame_uniform(scene, cam, look, r.size);
    if lines == Lines::Msaa {
        u.lines[2] = 0.0;
    }
    r.set_frame(gpu, &u);
    let mut enc = gpu.device.create_command_encoder(&Default::default());
    // RENDER_LOOK_SIL_VS=1 draws silhouettes through the vertex-cull path (fixed instance order)
    // instead of the atomic compaction, whose append order varies run to run.
    let parts = match look.sil {
        look::Sil::Geometric if std::env::var_os("RENDER_LOOK_SIL_VS").is_some() => Parts { sil_vertex_cull: true, ..Parts::ALL },
        look::Sil::Geometric => Parts::ALL,
        look::Sil::Fragment => Parts::DEFAULT,
    };
    // The baseline's lines are always opaque, like today's LineMaterial and LineBasicMaterial.
    let parts = Parts { a2c: lines != Lines::Blend || !look.quality, ..parts };
    render::encode(r, &mut enc, None, look.quality, true, parts, look.palette.canvas.linear4());
    gpu.queue.submit([enc.finish()]);
    readback(gpu, &r.output, r.size)
}

fn main() -> Result<(), String> {
    let root = repo();
    let tokens_css = root.join("src/styles/tokens.css");
    let out = Path::new(env!("CARGO_MANIFEST_DIR")).join("out");
    std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
    let gpu = Gpu::new()?;
    println!("adapter: {}  timestamps: {}", gpu.adapter, gpu.timestamps);
    let sampler = gpu.device.create_sampler(&wgpu::SamplerDescriptor {
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::MipmapFilterMode::Linear,
        ..Default::default()
    });
    let t0 = Instant::now();
    let ibl = ibl::Ibl::build(&gpu, &sampler);
    println!("ibl prefilter (one-time, incl. pipeline creation): {:.1} ms", t0.elapsed().as_secs_f64() * 1e3);

    for (name, dir) in [("nema", "spikes/fixtures/nema17"), ("fillet", "spikes/fixtures/fillet-block")] {
        let t0 = Instant::now();
        let scene = Scene::load(&root.join(dir))?;
        println!(
            "\n{name}: {} bodies, {} triangles, {} vertices, {} faces, {} edges ({} segments), {} silhouette candidates; load+prep {:.0} ms",
            scene.bodies,
            scene.triangles(),
            scene.positions.len(),
            scene.faces,
            scene.edges,
            scene.segments.len(),
            scene.silhouettes.len(),
            t0.elapsed().as_secs_f64() * 1e3
        );
        let (agree, total) = scene.winding_agreement();
        println!("  winding vs normals: {agree}/{total} triangles agree");
        let cam = Camera::iso_fit(scene.bbox_min, scene.bbox_max, SIZE[0] as f32 / SIZE[1] as f32);
        println!(
            "  camera: eye {:?} target {:?} distance {:.1} mm, grid minor {} mm",
            cam.eye,
            cam.target,
            cam.distance,
            look::grid_step(cam.distance)
        );
        for theme in [Theme::Dark, Theme::Light] {
            let palette = || Palette::load(&tokens_css, theme);
            let grid = look::baseline_grid(&cam, &palette()?);
            let t1 = Instant::now();
            let r = Renderer::new(&gpu, &scene, &ibl, SIZE, Some(&grid));
            gpu.wait();
            println!("  renderer setup ({}): {:.0} ms", theme.name(), t1.elapsed().as_secs_f64() * 1e3);
            let base = Look { quality: false, theme, palette: palette()?, sil: look::Sil::Fragment };
            let qual = Look { quality: true, theme, palette: palette()?, sil: look::Sil::Geometric };
            let bi = render_once(&gpu, &r, &scene, &cam, &base, Lines::Blend);
            let qi = render_once(&gpu, &r, &scene, &cam, &qual, Lines::Blend);
            write_png(&out.join(format!("baseline_{name}_{}.png", theme.name())), &bi)?;
            write_png(&out.join(format!("quality_{name}_{}.png", theme.name())), &qi)?;
            // Alternatives: the same look with lines that skip blending (see bench: blending is the
            // edge cost).
            let ai = render_once(&gpu, &r, &scene, &cam, &qual, Lines::A2c);
            let mi = render_once(&gpu, &r, &scene, &cam, &qual, Lines::Msaa);
            write_png(&out.join(format!("alt_a2c_{name}_{}.png", theme.name())), &ai)?;
            write_png(&out.join(format!("alt_msaa_{name}_{}.png", theme.name())), &mi)?;
            if name == "nema" {
                // Screw hole, label and lamination edges at 2x (nearest): blended | a2c | MSAA only.
                let (x, y, w, h) = (1180, 880, 440, 260);
                let c = |img: &Image| crop(img, x, y, w, h);
                let strip = side_by_side(&side_by_side(&c(&qi), &c(&ai)), &c(&mi));
                write_png(&out.join(format!("lines_{name}_{}.png", theme.name())), &zoom(&strip, 2))?;
            }
            write_png(&out.join(format!("compare_{name}_{}.png", theme.name())), &side_by_side(&half(&bi), &half(&qi)))?;
            let (cx, cy) = (SIZE[0] / 2 - 400, SIZE[1] / 2 - 300);
            write_png(
                &out.join(format!("crop_{name}_{}.png", theme.name())),
                &side_by_side(&crop(&bi, cx, cy, 800, 600), &crop(&qi, cx, cy, 800, 600)),
            )?;
            if name == "nema" {
                // Silhouette methods side by side on the shaft (3x, nearest): baseline (none),
                // geometric lines (default), fragment (alternative).
                let frag = Look { sil: look::Sil::Fragment, ..qual_clone(&qual) };
                let fi = render_once(&gpu, &r, &scene, &cam, &frag, Lines::Blend);
                let (x, y, w, h) = (1240, 420, 160, 380);
                let strip = side_by_side(&side_by_side(&crop(&bi, x, y, w, h), &crop(&qi, x, y, w, h)), &crop(&fi, x, y, w, h));
                write_png(&out.join(format!("silhouettes_{name}_{}.png", theme.name())), &zoom(&strip, 3))?;
            }
            // RENDER_LOOK_NO_BENCH=1 skips the timings (quick image-determinism runs).
            if name == "nema" && theme == Theme::Dark && std::env::var_os("RENDER_LOOK_NO_BENCH").is_none() {
                bench::run(&gpu, &r, &scene, &cam, &base, &qual, SIZE);
            }
        }
    }
    Ok(())
}
