# MESH1 fixtures for the native UI spikes

These are real MESH1 blobs from the OneCAD stack: the app crate's `WorkerManager` and `DocumentRuntime` drive the C++ OCCT 8.0.1 worker over OCW1. No mesh in this directory was made by hand. Each `.mesh1` file holds exactly the bytes `DocumentRuntime::get_mesh(body, Lod::Fine, None)` returns. That is the same call and the same tier (`DISPLAY_LOD = Lod::Fine` in `src-tauri/src/document_runtime.rs`) the app uses for committed geometry. By contract (`protocol/mesh_format.md` §6.2), Rust forwards the worker's Tessellate bytes without changing them.

| Fixture | What it is | Bodies | Triangles | Edges | Bytes |
|---|---|---:|---:|---:|---:|
| `nema17/` | `STEP/Nema17 Stepper Motor 17Hs4023.step`, imported through the app's STEP lane | 67 | 381,524 | 5,077 | 12,559,300 |
| `fillet-block/` | A parametric part: a constrained 40×30 rectangle sketched on XY, extruded 10 (blind, new body), then a fillet of r=3 on one vertical edge | 1 | 156 | 15 | 7,628 |

Each fixture directory holds:

- One `<bodyId>.mesh1` file per body. `bodyId` is the core `BodyId` uuid.
- A `manifest.json` file.

The manifest has a header with the source, the pipeline, `meshFormat`, `mesh1Version`, `lod`, `upAxis: "+Z"`, `units: "mm"`, `bodyCount` and `sceneBbox`. Each `bodies[]` entry then lists:

- `bodyId`, `wireId` (the OCW1 `body_<uuid>[:k]` string), `name` and `file`
- `byteLen` and `sha256`
- `bodyColor` and `faceColors`
- `flags`, `lod`, and the vertex, triangle, face, edge and edgePoint counts
- `bbox`, and the full `sections[]` table (type, name, offset, byteLen)

## How to regenerate

```bash
cd spikes/mesh-fixture
cargo run            # defaults: --worker ../../worker/build/onecad-worker
                     #           --out ../fixtures
                     #           --step "../../STEP/Nema17 Stepper Motor 17Hs4023.step"
```

The generator is a standalone cargo project with an empty `[workspace]` table. It takes a path dependency on `src-tauri` (`onecad_lib`), `onecad-core` and `onecad-protocol`. Its `Cargo.lock` is a copy of `src-tauri/Cargo.lock`, so dependency versions match the app. The app crate's build script needs the staged sidecar in `src-tauri/binaries/`. A run takes about 10 s with the debug build. Before writing new blobs, the generator deletes the old `*.mesh1` files in each fixture directory.

Every blob goes through two checks before it is written:

1. `onecad_protocol::mesh::validate_mesh_blob`, which checks the header, the section bounds, alignment, order and duplicate section types.
2. Count checks from `mesh_format.md` §4: POSITIONS and NORMALS are 12·V bytes, INDICES is 12·T, FACE_RANGES is 8·F, FACE_ID_OFFS is 4·(F+1), EDGE_RANGES is 8·E, EDGE_POSITIONS is 12·P, and every index is below V.

Each regen must publish, with no failed step and no NeedsRepair.

Pipelines:

- **nema17:** `imports::prepare_import` → `DocumentRuntime::add_import_record` → `run_regen(ToEnd{from:0})` → `get_mesh` for every head body.
- **fillet-block:** Sketch, then Extrude, then regen. The edge with the largest Z extent is picked from the box's MESH1 (`e:1`, the vertical edge at world (0,0)). `QueryElement` gives its anchor, and `promote_selection` mints its ElementId. Then Fillet, regen, and `get_mesh`.

## What was observed (2026-10-17)

- All 68 blobs pass the validator and the count checks. A separate Python parser read every file and found the counts in the manifest, contiguous face ranges, and ranges that cover all triangles.
- In every blob, `mesh1Version` is 1, `lod` is 2 (fine) and `flags` is `0x0003` (HAS_NORMALS and HAS_EDGES). No blob has FACE_BBOXES or FACE_COLORS. The id tables hold TopoKeys only (`f:1`, `e:1`, …), not ElementIds.
- **fillet-block:** the mesh volume is 11,980.663 mm³. The analytic value is 40·30·10 − 3²·(1−π/4)·10 = 11,980.686 mm³. The mesh has 7 faces (6 planar faces plus 1 rolled face). Its bbox runs from (−30, 0, 0) to (0, 40, 10). OneCAD's XY sketch plane has a **non-standard basis**: sketch +X maps to world +Y and sketch +Y maps to world −X (`onecad-core/src/sketch/plane.rs`). That is why the 40 mm side runs along world Y.
- **nema17:**
  - The STEP file holds 67 solids, and the import produced 67 bodies. The import reported one diagnostic, `STEP_HEALED` (a tolerance-level ShapeFix touch-up).
  - The scene bbox runs from about (−27.6, −32.8, −0.06) to (27.9, 27.6, 61.1).
  - Every body is named `Open CASCADE STEP translator 7.6 1`. That is the file's only `PRODUCT` name, and the app would show the same name.
  - The file has no `COLOUR_RGB`, so `bodyColor` is null and `faceColors` is empty. For a colored STEP case, use `src-tauri/tests/fixtures/colored_boxes.step`; this generator does not produce it.
  - The largest blob is 2.86 MB (70,042 vertices and 70,896 triangles).
- **Stability:** three back-to-back runs gave the same set of sha256 values for both fixtures. The fillet-block body id is fixed (`00000000-0000-0000-0000-00000000f101`). The nema17 body ids are **new on every run**, because the import record id is a random uuid. Match nema17 bodies by `sha256` or by order, not by id.

## MESH1 layout (summary of `protocol/mesh_format.md`)

All fields are little-endian. All offsets are bytes from the start of the blob. Every section offset is 4-byte aligned. Positions are **Z-up and right-handed**, in mm. Upload them to the GPU as they are, with no axis swap and no root rotation.

The header is 64 bytes:

| Off | Type | Field |
|---:|---|---|
| 0x00 | u32 | magic `0x4D455348` (stream bytes `48 53 45 4D`) |
| 0x04 | u16 | version = 1 |
| 0x06 | u16 | flags: bit0 HAS_NORMALS, bit1 HAS_EDGES, bit2 HAS_FACE_BBOXES, bit3 IDS_HAVE_ELEMENTIDS, bit4 HAS_FACE_COLORS |
| 0x08 | u32 | V, the vertex count |
| 0x0C | u32 | T, the triangle count |
| 0x10 | u32 | F, the face count |
| 0x14 | u32 | E, the edge count |
| 0x18 | u32 | P, the total number of edge polyline points |
| 0x1C | u16 | lod (0 = coarse, 1 = medium, 2 = fine) |
| 0x1E | u16 | sectionCount |
| 0x20 | f32×6 | bbox minX, minY, minZ, maxX, maxY, maxZ |
| 0x38 | u32×2 | reserved, always 0 |

The section table starts at `0x40` and has `sectionCount` entries of 16 bytes each, sorted by offset. Each entry is `{u32 type, u32 pad=0, u32 offset, u32 byteLen}`. Look sections up by `type`, never by position, and skip types you do not know.

| type | Section | Contents |
|---:|---|---|
| 1 | POSITIONS | f32×3×V, the vertex xyz |
| 2 | NORMALS | f32×3×V, smoothed per-vertex normals (present when flags bit0 is set) |
| 3 | INDICES | u32×3×T, the triangles, **grouped by face** |
| 4 | FACE_RANGES | `{u32 firstTri, u32 triCount}` × F, in triangle units |
| 5 | FACE_ID_OFFS | u32 × (F+1). These are prefix sums: face `i`'s id is the bytes `chars[offs[i]..offs[i+1]]` |
| 6 | FACE_ID_CHARS | UTF-8 face ids (`f:N`) |
| 7 | EDGE_RANGES | `{u32 firstPoint, u32 pointCount}` × E, in point units |
| 8 | EDGE_POSITIONS | f32×3×P, the polyline points, grouped by edge. Draw each edge as a line strip |
| 9 | EDGE_ID_OFFS | u32 × (E+1), prefix sums |
| 10 | EDGE_ID_CHARS | UTF-8 edge ids (`e:N`) |
| 11 | FACE_BBOXES | f32×6×F (optional; not present in these fixtures) |
| 12 | FACE_COLORS | u8×4×F sRGB. Alpha 0 means unset (not present in these fixtures) |

Here is the actual section table of `fillet-block/00000000-0000-0000-0000-00000000f101.mesh1`. It has V=170, T=156, F=7, E=15 and P=92, and its length is 7,628 bytes:

| Section | offset | byteLen |
|---|---:|---:|
| POSITIONS | 224 | 2040 |
| NORMALS | 2264 | 2040 |
| INDICES | 4304 | 1872 |
| FACE_RANGES | 6176 | 56 |
| FACE_ID_OFFS | 6232 | 32 |
| FACE_ID_CHARS | 6264 | 21 |
| EDGE_RANGES | 6288 | 120 |
| EDGE_POSITIONS | 6408 | 1104 |
| EDGE_ID_OFFS | 7512 | 64 |
| EDGE_ID_CHARS | 7576 | 51 |

The data starts at `0x40 + 16·sectionCount` (224 for 10 sections). Do not hardcode that number; always read the offsets from the table.

To parse a blob in a renderer:

1. Read V, T, F, E and P from the header.
2. Scan the table for the section types you need.
3. Take `&[f32]` or `&[u32]` views at `offset`. Every offset is 4-aligned, so `bytemuck::cast_slice` works on an aligned copy.
4. Bind POSITIONS, NORMALS and INDICES directly as vertex and index buffers.

To find which face a triangle belongs to, binary-search FACE_RANGES. Decode an id string only when it is picked.

## Not covered

- Raw OCW1 `Tessellate` replies were not captured byte for byte. The blobs come out of `DocumentRuntime::get_mesh`, which goes through `MeshCache`. They match the worker's bytes only by the documented forward-verbatim contract; this run did not compare them separately.
- There is no fixture with FACE_COLORS, FACE_BBOXES or ElementId tables (flags bits 2–4), and no coarse or medium LOD fixture.
