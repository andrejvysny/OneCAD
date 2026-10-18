# SolidCraft viewport source

This subtree vendors the complete reusable `solidcraft-geom`, `solidcraft-render`, and
`solidcraft-gpu` source, plus the viewport WGSL asset required by `include_str!`.
It does not vendor SolidCraft's engine, kernel, document, sketch, I/O, or UI crates.
OneCAD keeps OCCT/PlaneGCS and its own document, scene, topology, and picking authority.

Source repository: https://github.com/tangency-labs/solidcraft.git
Source commit: `a228a5830eedf88856fd392b7db0ff5977f352d6`
License: MIT OR Apache-2.0; both original license texts accompany the source.
Copyright (c) 2026 SolidCraft contributors.
The upstream ATTRIBUTION.md identifies `assets/shaders/viewport.wgsl` as original
SolidCraft resource data or drawing code, under MIT OR Apache-2.0.
No external images, fonts, icons, studio media, or artwork are included.

## Local adaptations

- Replace workspace-inherited package attributes and dependencies with standalone
  manifests, preserving version 0.16.0, edition 2024, upstream dependency features,
  and local paths wholly inside this subtree. Pin wgpu to 30.0.1, matching OneCAD.
- Remove GPU development dependencies on solidcraft-kernel and solidcraft-engine.
- Exclude `crates/gpu/src/tests.rs`, `crates/gpu/tests/capture.rs`, and
  `crates/gpu/tests/depth.rs`, which depend on those excluded crates. Remove only
  their `mod tests` declaration in gpu/src/lib.rs. Pure geometry/render tests and
  inline GPU shader/picking tests remain.
- All production Rust and WGSL source is copied verbatim from the pinned commit.
  No path refers to the donor checkout; no symlinks are used.

The OneCAD adapter lives in `crates/onecad-render/src/solidcraft_viewport*.rs` outside
this third-party source. It uses the host device/queue and offscreen output, with
OneCAD capture validation, resource limits, preview generation checks, and queue fences.
Vendoring the full stack does not imply that every donor feature has corresponding
OneCAD UI or domain support. The original GPU APIs remain available to downstream hosts.

## Source refresh

Refresh all copied source from the pinned commit, reapply only the adaptations
above, and regenerate `SOURCE_SHA256SUMS`. Compared with the earlier
`6bde126a5cabd1f5af0c68e6955bd0176fab805a` checkpoint, 0.16.0 adds geometry
network, section and spline-space modules. GPU, render, viewport shader and
adapted gizmo/cube sources are unchanged between those checkpoints.
