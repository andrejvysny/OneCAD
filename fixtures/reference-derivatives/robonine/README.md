# Robonine faceted diagnostic derivatives

This retained file derives from the pinned Robonine clamp_1 simulation visual STL.
Copyright (c) 2025 Robonine; CERN-OHL-P-2.0. See
[licensing and notices](../../public-cad/robonine/LICENSING.md), the retained
[originals](../../public-cad/robonine/simulation/so_arm_101_description/meshes/visual/),
and the [input manifest](../../public-cad/manifest.json). They are diagnostic
inputs, not approved output goldens or physical assembly evidence.

OCP 8.0.1.1 constructed one planar face per original binary-f32 triangle, without
healing or merging faces, and retained the three oriented shells as one solid.
BinTools V4 stored no triangulation or normals. Same-kernel readback reported one
valid solid, three shells, 8,808 faces, 13,212 edges and 4,400 vertices per part.
This does not establish absence of self-intersection or independent kernel
qualification. Independent source bounds, signed triangle sums and q=0 poses
are in [the source oracle](../../reference-oracles/robonine-simulation.md).

| Output | Bytes | SHA-256 |
| --- | ---: | --- |
| clamp_1.brep | 3,245,816 | `df23f34e1e57dc4861fa69f4b893916eb39b793948012a4856019170ca5e5b28` |

To reproduce, use Python with OCP 8.0.1.1 installed in a virtual environment.
For `clamp_1`, copy the corresponding original STL to an
empty working directory as `<name>.stl`, then run:

```sh
python faceted.py <name> <working-directory>
python cavities.py <name> <working-directory>
python bintools.py <name> <working-directory>
```

Scripts check the pinned source size and SHA-256 and OCP version before each stage;
they refuse existing outputs. OCP exposes its package version here, but does not
expose a separate runtime OCCT version through these bindings. Run only in a new
scratch directory, never over retained fixtures. Scripts emit diagnostic JSON on
stdout. The cavity classification, distances and roundtrip are same-kernel
checks. Hashes identify these exact fixture bytes;
other OCP builds may serialize equivalent shapes differently. Current component
replay promotes every face and caps persistent identities at 4,096, so this
8,808-face part is expected to be refused. This does not complete ONECAD-531.
