# Render-look spike (ONECAD-259, Phase 0.4)

Status: built and measured on 2026-10-17, macOS only (the iPad target was dropped from the program the same day). Revised the same day after a fresh review (see "Revision after review"). Decision update, 2026-10-18: the user approved quality lighting and grid in both themes and delegated the line choice. Blended analytic lines were selected for visual continuity. Native qualification, determinism/tolerance policy, golden images and publication remain unverified; no goldens are frozen.

A headless wgpu 30.0.1 renderer (Metal, Apple M4 Pro) draws the two MESH1 fixtures at 2560×1600 from today's iso home camera in two looks:

- **baseline**: an approximation of today's Three.js viewport, with opaque 1-pixel-ish lines like today's `LineMaterial`.
- **quality**: the candidate native look. It adds PBR with a procedural studio IBL, GTAO, a ground contact shadow, analytic anti-aliased wide lines, silhouettes, an analytic infinite grid, reversed-Z depth and MSAA 4x.

Both looks render in the dark and the light theme. All colours come from `src/styles/tokens.css`, which the spike reads at runtime.

At 2560×1600 with the NEMA model (381,524 triangles, 69,267 edge segments), the GPU cost of a frame is:

| Look | Final run | Range over 5 runs |
|---|---:|---:|
| Baseline (opaque lines, like today) | 0.47 ms | 0.467–0.501 ms |
| Quality (blended analytic lines, the default) | 1.77 ms | 1.760–1.849 ms |
| Quality, including a contact-map rebuild (needed only when the geometry changes) | 2.03 ms | 2.029–2.182 ms |
| Quality with MSAA-only lines (alternative) | 1.09 ms | 1.084–1.152 ms |
| Quality with alpha-to-coverage lines (alternative) | 1.14 ms | 1.136–1.204 ms |

The shading techniques are cheap. **Almost all of the quality line cost is in blended line fragments**: the same edge quads drawn without blending cost about 0.06 ms instead of about 0.80 ms. On this tile-based GPU that difference also includes hidden-surface removal, which blending disables (see "Where the line cost goes"). Two non-blended line variants are rendered next to the default so you can judge them.

## How to run

```bash
cd spikes/render-look
cargo run --release > out/run.txt   # writes out/*.png, prints counts and timings
cargo clippy --release -- -D warnings
```

The crate is standalone: it has an empty `[workspace]` table and its own `target/`. Its only dependencies are wgpu, pollster, bytemuck, glam and png. The output of the final run behind this document is `spikes/render-look/out/run.txt` (not `.log`, which the repository `.gitignore` drops). Switches:

- `RENDER_LOOK_NO_BENCH=1` skips the timings (about 6 s per run, used for the determinism runs).
- `RENDER_LOOK_SIL_VS=1` draws the silhouettes through the vertex-cull path instead of the compute compaction.
- `RENDER_LOOK_TS_DEBUG=1` prints raw timestamp ticks.

Every PNG is checked before it is written: a pixel with alpha below 255 fails the run.

## Output images (`spikes/render-look/out/`)

| File | What it shows |
|---|---|
| `baseline_{nema,fillet}_{dark,light}.png` | Today's look approximated: 2560×1600 |
| `quality_{nema,fillet}_{dark,light}.png` | The candidate look: 2560×1600. Quality lighting/grid approved 2026-10-18; blended analytic line choice delegated and selected |
| `alt_msaa_{nema,fillet}_{dark,light}.png` | The quality look with MSAA-only lines (hard coverage, no blending): 2560×1600 |
| `alt_a2c_{nema,fillet}_{dark,light}.png` | The quality look with analytic coverage through alpha-to-coverage (no blending): 2560×1600 |
| `compare_*` | Baseline on the left and quality on the right, each at half size |
| `crop_*` | The central 800×600 at 1:1, baseline on the left and quality on the right. Use these to judge line anti-aliasing and AO |
| `lines_nema_{dark,light}.png` | Screw hole, label and lamination edges at 2x (nearest), left to right: blended (default), alpha-to-coverage, MSAA-only |
| `silhouettes_nema_{dark,light}.png` | The motor shaft at 3x (nearest), left to right: baseline (no silhouettes), geometric silhouettes (the default), fragment silhouettes (the alternative) |

## Pass ladder (quality)

| # | Pass | Target | When it runs |
|---|---|---|---|
| 1 | contact height | Depth32Float 1024², ortho looking up | when the geometry changes |
| 2 | contact occlusion | Rg16Float 256² (4×4 box down-sample) | when the geometry changes |
| 3–4 | contact blur h / v | Rg16Float 256² | when the geometry changes |
| 5 | depth/normal prepass | Depth32Float plus Rgba16Float view normals, 1 sample | every frame |
| 6 | GTAO | R16Float, full resolution | every frame |
| 7–8 | AO bilateral blur h / v | R16Float | every frame |
| 9 | silhouette compaction | compute shader writing segments and indirect args | every frame |
| 10 | main pass, MSAA 4x | Rgba8UnormSrgb plus Depth32Float, ×4 samples, resolved in-pass | every frame |

The main pass draws, in order:

1. bodies (PBR)
2. the ground quad (grid and contact shadow)
3. silhouette lines (indirect draw)
4. feature edges

The baseline runs only pass 10. It draws the bodies with three.js-equivalent lighting, today's 201+201 grid lines, and the feature edges, all lines opaque.

Tone mapping happens in the body shader, as three.js does it. The colour buffer therefore holds display-referred linear values, and lines, grid and background write their token colours unchanged. The MSAA resolve is the hardware average in linear light. Writing to an `…Srgb` target encodes the output.

## Techniques and parameters

**Geometry.** MESH1 POSITIONS, NORMALS and INDICES are merged into one vertex and index set for all 67 bodies. They are uploaded verbatim: Z-up, no axis swap, no root transform. In NEMA, 381,503 of 381,524 triangles have a counter-clockwise winding that agrees with their vertex normals; in the fillet block, 156 of 156 do. Back-facing fragments are therefore shaded from their other side using `front_facing`, which is three's `DoubleSide` behaviour.

**Camera.** Today's home view:

- `ISO_YAW` is −45° and `ISO_PITCH` is atan(1/√2), Z-up.
- Vertical field of view 76°, near plane 0.1 mm.
- The framing uses the persp branch of `computeCameraFit` with margin 1.15.
- The projection is reversed-Z with an infinite far plane: depth = near / distance.
- The NEMA camera sits 89.8 mm from the target; the fillet-block camera 47.0 mm.

**Depth.** Depth32Float, cleared to 0, compare `Greater`. Faces carry a depth bias of 8 constant steps plus a slope of 1.0 pixel, the same value as SolidCraft's `FACE_BIAS`, negated for reversed depth. Lines draw at their own depth and never write depth. The baseline uses three's `polygonOffset(1, 1)`.

**Shading (quality).** Metallic/roughness PBR: GGX distribution with Smith height-correlated visibility, Lambert diffuse and the split-sum IBL. Parameters:

- base colour: `--color-body-fill`
- metallic 0, f0 0.04, roughness 0.42
- exposure 0.95 (dark) and 0.78 (light), followed by Khronos PBR Neutral

AO is applied in three ways:

- diffuse image-based light through the GTAO multi-bounce fit
- specular image-based light through Lagarde specular occlusion
- the key light at 35% strength

**Studio IBL** (procedural, no HDRI). The radiance of a light-grey cyclorama goes from 0.16 at the floor to 0.80 overhead, plus four lights:

- a key softbox: azimuth +40°, elevation 52°, 48°×30°, radiance 7
- a fill strip: −80°, 18°, radiance 2.2
- a rim strip behind the part: 180°, 22°, radiance 3
- a top light: radiance 2

It is prefiltered once:

- a 512² cube with a full mip chain
- a 256² specular cube with 6 roughness mips: 512 GGX samples each, with filtered importance sampling
- a 32² irradiance cube
- a 128² BRDF lookup table

The prefilter, including its pipeline creation, took 38–63 ms per process over the five full runs (55.0 ms in the final run).

The lookup direction turns with the camera yaw only, about world Z. This keeps today's camera-relative lighting cue without tilting with pitch, and the geometry is never rotated.

One directional key light, intensity 1.0, sits on the softbox direction to give crisp highlights.

**GTAO.** It follows Jimenez et al. 2016 and the structure of XeGTAO:

- 4 slices with 6 steps per side, at full resolution
- world radius 3% of the scene diagonal (3.07 mm for NEMA), capped at 256 px
- falloff over the outer 61.5% of the radius
- final power 1.8, interleaved-gradient-noise jitter
- 2×9-tap separable blur, weighted by depth with a relative tolerance of 1%

There is no temporal accumulation, so the noise is handled by spatial filtering alone.

**Contact shadow.** The ground quad lies at Z = 0, today's grid plane. An ortho depth pass from below records, for each ground texel, the height of the lowest surface above it. Let E be the larger footprint extent; the map covers ±0.85E. Two occlusion terms are derived and blurred separably:

| Term | Height range | Blur sigma | Shape |
|---|---|---|---|
| tight | 0.03E | 0.012E | (1−h)³ |
| soft | 0.5E | 0.12E | (1−h)² |

The ground multiplies the canvas colour by 1 − strength·occlusion. Strength is 1.0 in dark and 0.55 in light; the tight term has weight 0.5. The map depends only on the geometry, so an app rebuilds it on a model change, not on a camera move. In dark the effect is necessarily subtle: the darkest floor pixel is luma 25 (NEMA) and 27 (fillet block) against a canvas of 38 (Pillow `L`; measured before the review, and the quality images have not changed since apart from the run-to-run pixels described under Determinism).

**Feature lines.** One instance per MESH1 `EDGE_POSITIONS` segment (69,267 for NEMA). The quad expansion and near-plane clipping are adapted from SolidCraft's `vs_line`, with these changes:

- 4-vertex triangle strips instead of 6-vertex lists
- round caps, from the distance to the segment
- no `discard`: uncovered fragments output alpha 0, which leaves the pixel unchanged when blended and gives an empty sample mask under alpha-to-coverage

The fragment shader has two coverage modes:

- **Analytic** (quality): the overlap of the 1 px pixel footprint with the line band, plus a 1 px quad margin for the falloff. Blended, or turned into the sample mask by alpha-to-coverage.
- **Hard** (baseline and MSAA-only): the quad is exactly the line band, and inside the segment's span the fragment outputs full coverage, so the rasteriser's 4 samples anti-alias the long sides. Only the round-cap regions beyond the endpoints are cut, with one distance test at the pixel centre. This matches three's `LineMaterial`, which discards only in the cap region. Hard lines always go through alpha-to-coverage, so with coverage 0 or 1 they are opaque.

Measured with the discard variant (bench row "edges blended + discard uncovered"): the quality main pass goes from 1.04 ms to 1.27 ms. A shader that may discard defers the depth test, so hidden edges get shaded.

Line widths:

- quality: 2.2 device px (1.1 CSS px at 2x)
- baseline: 2.5 px, hard, opaque, MSAA only: three's `LineMaterial` at 1.25 CSS px

The colour is `--color-body-edge`.

**Silhouettes (default: geometric).** On the CPU, every interior edge of a curved face is collected together with the geometric normals of its two triangles. Vertices are welded per face at 0.1 µm, so seams of periodic faces close, and coplanar pairs are dropped. NEMA yields 339,992 candidates, sorted so the candidate buffer is the same in every process.

Each frame, a compute pass keeps the edges whose two triangles straddle the terminator for the current eye. At the home view that is 3,621 edges. They are appended to a buffer and drawn indirect with the same line pipeline, so silhouettes match the edge weight and anti-aliasing exactly. Drawing them adds about 0.01 ms to the main pass.

Two alternatives were measured:

- **Culling in the line vertex shader** over all candidates: the main pass costs 1.05–1.18 ms against 1.04–1.15 ms with the compaction, and the compaction itself costs 0.05 ms. The two are about equal in total, and this path draws in a fixed order, so it is deterministic (see Determinism).
- **SolidCraft's fragment silhouette.** The body shader darkens pixels where the smooth normal turns edge-on. It costs nothing extra, but on NEMA's irregularly tessellated shaft it speckles, while the geometric lines stay continuous. See `silhouettes_nema_light.png`.

**Grid (quality).** An analytic grid on one quad on the world XY plane, using today's `GridPlane` rules:

- step `snapToDecade(distance/25)` on the 1/2/5/10 ladder, majors every 10 minors (5 mm minor for NEMA, 2 mm for the fillet block)
- extent 100 minor cells
- fade centre snapped to a major line
- lines: minor 1.0 px and major 1.5 px, with coverage computed from `fwidth`
- radial fade (1 − r/R)²
- minor lines fade out where a cell is smaller than about 8 px, which removes the horizon moiré visible in the baseline

The grid is depth-tested against the bodies and writes no depth.

**Theme.** At runtime the spike parses `@theme {…}` (light) and `:root[data-theme="dark"] {…}` from tokens.css. It reads `--color-canvas`, `--color-viewport-grid-minor`, `--color-viewport-grid-major`, `--color-body-fill` and `--color-body-edge`: the same tokens `src/viewport/engine/palette.ts` uses for the clear colour, grid, body and edges. No hex literal lives in the spike.

**Baseline emulation.** It uses the `LIGHT_LEVELS` WebGL table:

| Theme | Env | Key | Fill | Hemi |
|---|---:|---:|---:|---:|
| dark | 0.3 | 2.1 | 0.7 | 0.15 |
| light | 0.35 | 1.9 | 0.6 | 0.25 |

The lights follow the `lightRig` offsets: key at yaw+40° and pitch+35° (floor 20°), fill at yaw−75° and pitch−15°. The hemisphere light's ground colour is the canvas. The material is roughness 0.5 and metalness 0, with NeutralToneMapping. Edges and grid lines are opaque, as today's materials are (`bodyMaterials.ts` edges are not transparent; the grid is a `LineBasicMaterial`); the grid is drawn as 1 px hard quads, standing in for GL lines.

It deviates from today in three ways:

- The spike's studio environment stands in for three's `RoomEnvironment`, which was not reproduced.
- Edge widths assume a DPR of 2.
- Grid lines are 1 px MSAA quads, not GL line primitives.

`OneCAD.png` shows near-white bodies rather than the tinted `--color-body-fill`, and it shows the connector on the other side, so it is not the same camera or code path. The baseline follows the token and the home camera, not the screenshot.

## Where the line cost goes

Each row is the quality main pass with all parts on (bodies, ground, silhouettes, edges). Every row is a bench row printed in `out/run.txt` and reproducible with `cargo run --release`. The three experiment rows use pipelines that exist only for this attribution; no image uses them.

| Feature-edge variant | Main pass, final run | Range over 5 runs | Edges' share |
|---|---:|---:|---:|
| Default: analytic coverage, alpha-blended | 1.038 ms | 1.038–1.146 | about 0.80 ms |
| Experiment: every edge quad culled after the full vertex work | 0.289 ms | 0.288–0.305 | about 0.05 ms |
| Experiment: the same quads opaque (blending off; wrong image) | 0.298 ms | 0.298–0.320 | about 0.06 ms |
| Experiment: blended, uncovered fragments discarded | 1.265 ms | 1.265–1.406 | about 1.03 ms |
| Alternative: analytic coverage through alpha-to-coverage (`alt_a2c_*`) | 0.401 ms | 0.401–0.441 | about 0.17 ms |
| Alternative: hard coverage, MSAA only (`alt_msaa_*`) | 0.357 ms | 0.356–0.380 | about 0.12 ms |

The "edges' share" subtracts the main pass without edges (0.236 ms). So the per-instance vertex work for 69,267 segments is small, and rasterising the quads is small. Blended line fragments are what cost: 48,657 of the segments are shorter than 1 px on screen (threads, the lamination stack, the label text), and their small overlapping quads total 1.8 Mpx before the depth test.

One confound: on this tile-based GPU, opaque fragments let hidden-surface removal skip fragments that a later opaque fragment overwrites, and blending turns that off and forces each fragment to read and write its samples in order. The 0.80 ms therefore measures "blended line fragments on Apple TBDR", not blend arithmetic alone. On an immediate-mode desktop GPU the split may differ; it was not measured.

The same holds for the baseline. Its main pass costs 0.467 ms with opaque lines and 0.844 ms when its lines go through the blended pipeline (the previous version of this spike). Its parts are bodies 0.156 ms, grid 0.19 ms (402 screen-spanning lines) and edges 0.12 ms.

The two non-blended variants:

- **Alpha-to-coverage** (`alt_a2c_*`): the analytic coverage becomes the 4-sample mask. At 2x zoom (`lines_nema_*`) it is hard to tell from the blended default; edges keep the same weight.
- **MSAA-only** (`alt_msaa_*`): hard coverage at the exact line width, anti-aliased by the 4 MSAA samples along the sides. It reads almost the same as the other two at 2x zoom.

At 1:1 the three are hard to tell apart. Against the blended default, 32,703 (a2c, dark) to 36,471 (MSAA-only, light) of 4.1 M pixels differ, 13,702–19,213 of them by more than 8 of 255 levels, all along lines. The mean luma of the label region (x 1460–1620, y 940–1140) is about 1.3 levels lighter in both variants. Both variants are deterministic (next section). Making MSAA-only the default would cut the quality frame from about 1.77 ms to about 1.09 ms on this model.

## Determinism (same GPU, same binary)

All PNG files were compared byte for byte (`cmp`, which covers every RGBA channel) across 13 runs of the final code: 8 image-only runs and 5 full runs with the bench. Pixel counts come from a full-RGBA numpy diff.

- **Byte-identical in all 13 runs:** every baseline image, every fillet-block image, `alt_a2c_*`, `alt_msaa_*` and `lines_nema_*`.
- **Not byte-identical:** `quality_nema_*`, and the images that contain a crop of it (`compare_nema_*`, `crop_nema_*`, `silhouettes_nema_*`). Between two runs, 48–94 of 4.1 M pixels of `quality_nema_*` differ, by at most 2 of 255.
- **Cause, verified:** the silhouette compaction appends segments with an atomic counter, so their draw order changes from run to run. Blending into an 8-bit target rounds differently depending on that order. With `RENDER_LOOK_SIL_VS=1`, which draws in a fixed instance order, two runs of the final code gave byte-identical images for every file.

What the review found, and what changed:

- The earlier `alt_a2c_*` and `lines_nema_*` images varied run to run only in their alpha channel, which was itself a defect (see "Revision after review"). With alpha no longer written, they are identical in all 13 runs.
- The reviewer saw `baseline_nema_light` differ once in 8 runs of the earlier code (1,235 px, max 7), with no cause found. In 18 runs of the current baseline code (13 default plus 5 with `RENDER_LOOK_SIL_VS=1`), every baseline image was byte-identical. The baseline line pipeline changed in between, so this neither reproduces nor explains that observation. It stays open.

For golden images, compare with a small tolerance, for example at most 2 levels per channel on at most 0.01% of pixels, rather than exact bytes. The one unexplained baseline difference is the reason. A deterministic compaction (a prefix-sum scan instead of an atomic append), or the vertex-cull path, which costs about the same, removes the known source.

## Measurements (NEMA, 2560×1600, Apple M4 Pro, Metal)

**Method.** Each pass, or each whole frame, is recorded 40 times into one command buffer. The wall time from submit to done, divided by 40, is taken as the cost; the figure is the median of 11 submissions. A single-frame submit-to-done latency (median of 60 frames after 10 warm-up frames) is also printed.

The table gives the final run (`out/run.txt`) and the range over the 5 full runs of the final code. The first two of those ran while the machine was busier (load average about 2.5–3.5) and give the high ends of the ranges. Other agents' work and normal desktop load shared the machine throughout, so this was not an isolated benchmark.

Timestamp queries are supported (`TIMESTAMP_QUERY`, at pass boundaries only) but are not usable for per-pass durations here:

- On the main pass, the end-of-fragment sample precedes the start-of-vertex sample in 46–60 of 60 frames.
- On the other passes, the deltas grow pass by pass like times since the start of the frame. In the final run: GTAO 0.53, AO blur h 0.61, AO blur v 0.68 ms.

They are printed in the log, but no figure below uses them.

| Pass | Final run | Range |
|---|---:|---:|
| depth/normal prepass | 0.099 ms | 0.098–0.100 |
| GTAO | 0.410 ms | 0.406–0.426 |
| AO blur h + v | 0.083 + 0.083 ms | 0.081–0.084 each |
| silhouette compaction (compute) | 0.052 ms | 0.051–0.054 |
| main pass, MSAA 4x | 1.037 ms | 1.030–1.146 |
| **quality frame, repeated as a whole** | **1.767 ms** | 1.760–1.849 |
| contact height + occlusion + blur h + blur v (only on a geometry change) | 0.094 + 0.049 + 0.090 + 0.089 ms | 0.094, 0.049, 0.087–0.090, 0.087–0.089 |
| quality frame including the contact rebuild | 2.031 ms | 2.029–2.182 |
| baseline frame (main pass only, opaque lines) | 0.467 ms | 0.467–0.501 |
| baseline main pass, bodies only | 0.156 ms | 0.156–0.163 |
| baseline main pass, bodies + grid | 0.350 ms | 0.348–0.365 |
| single-frame submit → done (baseline / quality / quality with rebuild) | 0.95 / 2.02 / 2.19 ms | 0.92–1.60 / 1.92–2.07 / 2.12–2.26 |

**Main pass, cumulative cost** (final run; each row adds one part):

| Main pass contents | ms |
|---|---:|
| bodies (PBR + IBL + AO) | 0.162 |
| + ground (grid and contact shadow) | 0.224 |
| + silhouette lines, compacted | 0.236 |
| + feature edges (the default look) | 1.038 |

**Counts.**

| Fixture | Bodies | Triangles | Vertices | Faces | Edges | Segments | Silhouette candidates |
|---|---:|---:|---:|---:|---:|---:|---:|
| NEMA | 67 | 381,524 | 289,946 | 2,035 | 5,077 | 69,267 | 339,992 |
| fillet block | 1 | 156 | 170 | 7 | 15 | 77 | 35 |

**Memory.** These figures are calculated from the formats, not measured:

| Resource | Size |
|---|---:|
| MSAA colour and depth | 2 × 65.5 MB |
| Prepass depth and normals | 16.4 MB and 32.8 MB |
| AO targets | 3 × 8.2 MB |
| Output | 16.4 MB |
| Contact height and occlusion maps | 4.2 MB + 3 × 0.26 MB |
| IBL | about 21 MB |
| NEMA silhouette candidates | 16.3 MB, plus 8.2 MB for the visible-segment buffer |

wgpu does not expose memoryless storage, so the discarded MSAA attachments still take memory. The candidate encoding could shrink to about 16 B per edge, as an index pair plus octahedral normals.

**Setup.** Renderer setup for NEMA, including pipeline creation, took 6–12 ms per theme over the five full runs. The first run after the shaders changed took 154 ms for the first NEMA renderer, a single observation of a partly cold Metal shader cache. A cold first launch after a clean build or a cleared cache was not measured.

## Findings and risks

1. **The cost is in blended line fragments, not in the lighting.** Lighting costs about 0.9 ms in total:
   - prepass, GTAO and blurs: 0.68 ms
   - PBR bodies: 0.16 ms
   - ground with grid and contact shadow: 0.06 ms

   Blended feature edges cost about 0.80 ms on their own, and non-blended edges about 0.12–0.17 ms. On Apple GPUs part of that gap is lost hidden-surface removal. Two levers remain open for big models:
   - screen-space edge decimation in a compute pass, merging sub-pixel runs of each polyline
   - edge LOD

   Neither was built.
2. **Hidden-line leakage on thin plates is a risk of the slope bias.** A slope-scaled bias of 1 px pushes a face back by up to about one pixel footprint of depth: about 0.09 mm at the NEMA camera distance for a face at 45°. Edges that lie closer than that behind a thin face can show through it. This session did not isolate a case. A per-edge classification, or a smaller slope plus per-pixel line depth, would remove it.
3. **Tessellation quality drives silhouette quality.** The shaft's MESH1 tessellation is irregular, and its shading is faceted in both looks. The geometric silhouettes handle it well; the fragment silhouettes speckle.
4. **Dense real edges read as dark masses** in both looks: threads, the lamination stack and the label text. A density-aware edge fade is not built.
5. **The blended quality look is not bit-reproducible** because of the atomic silhouette append (see Determinism). The baseline, MSAA-only and alpha-to-coverage outputs were identical over 13 runs of the final code, but an unexplained one-off baseline difference in the earlier code argues for tolerance-based golden images.

## Revision after review

A fresh reviewer checked the first version of this spike. The fixes, all re-run for this document:

- **Alpha-to-coverage images were partly transparent** (high). The a2c pipelines wrote the coverage into the target's alpha, so 37,534 pixels of `alt_a2c_nema_dark` had alpha below 255 (minimum 108), and viewers blended them with their own background. The "lighter, textured label letters" in the first version were this artifact. The a2c pipelines now write colour only (`ColorWrites::COLOR`), and `write_png` refuses any non-opaque pixel. The RGB of the a2c images did not change: against the earlier files, 1 pixel moved by more than 8 levels.
- **Hard lines dropped half of the MSAA ramp** (medium). The hard mode tested coverage once at the pixel centre, so covered samples in pixels whose centre lay outside the band were dropped. Hard lines now output full coverage inside the segment span and leave the sides to the rasteriser, cutting only the round caps at the pixel centre, as three does. `baseline_*` and `alt_msaa_*` were re-rendered; the lines are visibly smoother and no longer thinner.
- **The baseline drew its lines blended** (medium), although today's edges are opaque. The baseline edges and grid now go through alpha-to-coverage with hard coverage, which makes them opaque. The baseline frame dropped from 0.84–0.92 ms to 0.47–0.50 ms. The blended figure is kept as a bench row for comparison.
- **Determinism claims compared RGB only** (medium). The section above is rewritten from byte-level comparisons of 13 runs, and the unexplained baseline observation is listed as open.
- **`run.log` was gitignored** (low). It is now `out/run.txt`.
- **The edge-cost experiments could not be reproduced** (low). They are now bench rows backed by their own pipelines (`fs_line_discard`, `vs_edge_culled`, an opaque edge pipeline), and the TBDR confound is stated.
- **Comment and notices** (low). The `ground.wgsl` header now says Z = 0. The MIT copyright and permission notices for SolidCraft and XeGTAO are in `spikes/render-look/THIRD_PARTY_NOTICES.md`, which also points to the Apache-2.0 terms for the Khronos tone mapper. `ATTRIBUTION.md` already lists every adapted file and was not changed.

## Not done

- **No golden images and no CPU/software-tier render.** Appearance approval on
  2026-10-18 does not itself freeze goldens. Determinism/tolerance policy and
  native qualification remain unverified.
- **Not run on any other GPU or Mac.** All numbers come from one M4 Pro under normal desktop load. This was not an isolated benchmark.
- **Not built:**
  - selection and hover
  - section caps and clipping
  - transparency
  - edge classification (tangent or seam)
  - line LOD and decimation
  - a deterministic silhouette compaction
- **The baseline does not reproduce three's `RoomEnvironment`.** It is a lighting-math approximation, not a pixel match of the running app.
- **The one-off `baseline_nema_light` difference** the reviewer saw in the earlier code was not reproduced or explained.
- **The TBDR share of the blending cost** (lost hidden-surface removal against blend arithmetic) was not separated, and no immediate-mode GPU was measured.
- **Per-pass timestamp timing could not be used** on this driver (see Method). CPU-side repeated timing was used instead.
- **Cold-start pipeline compilation** was not measured (see Setup).

## Historical acceptance questions (2026-10-17)

The questions below are retained as investigation history. On 2026-10-18 the
user approved quality lighting/grid in both themes and delegated line choice;
blended analytic was selected for continuity. The existing spike line width and
silhouette treatment remain the implementation reference, not a newly verified
native golden. No new render or qualification was run for this decision update.

- Do you accept the quality look (dark and light), or should any part change: exposure, AO strength, contact-shadow strength, line width (2.2 px) or the grid?
- Which line treatment should Phase 3 build: blended analytic (`quality_*`, about 0.80 ms of edges on NEMA), MSAA-only (`alt_msaa_*`, about 0.12 ms) or alpha-to-coverage (`alt_a2c_*`, about 0.17 ms)? Compare them in `lines_nema_*.png`.
- Should silhouette lines stay the same weight as feature edges, or be thinner or lighter?
- Is camera-yaw-relative studio lighting (today's cue) the behaviour to keep?
