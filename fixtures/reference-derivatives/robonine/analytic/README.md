# Scoped analytic Robonine inputs

Copyright (c) 2025 Robonine. SPDX-License-Identifier: CERN-OHL-P-2.0.
The four XBF files are modified hardware sources: OneCAD extracted only four
single-solid leaf parts, baked each first occurrence's rigid placement into its
geometry, added explicit face labels, and serialized BinXCAF storage version 12.
No healing, simplification, fitted surfaces, or vendor bodies were added.

Source: [Robonine SO-ARM100/101 Parallel Gripper](https://github.com/roboninecom/SO-ARM100-101-Parallel-Gripper/tree/e7cb78a7a1cd073ea8f845486398a23549343c7b),
`models/RB9.01.062.000 Gripper.STEP`, 5,731,780 bytes,
SHA-256 `3d10caca74f4e82e8d0b26c0ef2254e12dc1495c027eba1eae207944521d2104`.
The complete mixed assembly is not redistributed here. Its selected leaves are
Robonine's `.010 Main frame`, `.020 Clamp`, `.030 Gear rack_m0.8`, and
`.040 Gear for gripper`. The upstream [licensing map](../../../public-cad/robonine/LICENSING.md),
[REUSE map](../../../public-cad/robonine/REUSE.toml),
[notice](../../../public-cad/robonine/NOTICE), and
[CERN-OHL-P-2.0 text](../../../public-cad/robonine/LICENSES/CERN-OHL-P-2.0.txt)
are retained. The hardware is supplied without warranty under that license.

| File | First occurrence | Product definition | Product | Faces |
|---|---|---|---|---|
| part-010.xbf | NAUO1 / #72211 | #57054 | #48712 | 181 |
| part-020.xbf | NAUO2 / #72212 | #846 | #34184 | 168 |
| part-030.xbf | NAUO3 / #72213 | #24442 | #16015 | 108 |
| part-040.xbf | NAUO4 / #72214 | #32962 | #38241 | 96 |

`observations.json` pins each retained output's size and SHA-256. It records OCP
8.0.1.1 readback diagnostics, including adaptive volume integration with epsilon
1e-9. These are same-kernel extraction observations, not independent geometry
oracles. The STEP contains no supplier-authored mass, area, or volume properties.
Do not compare its adaptive volumes to nonadaptive worker values as exact equals.

Each file's local frame is explicitly chosen as the source assembly's root frame
with the first occurrence geometry baked in; its first authored pose is identity.
A second clamp and rack use an explicitly authored root Z rotation of pi. This
choice avoids inferring a unique STL-to-CAD pose. It does not establish physical
fit, joint origins, travel limits, contact, or synchronized rack-and-pinion motion.
These are imported reference parts, not native feature histories.

To reproduce in a virtual environment containing OCP 8.0.1.1:

```sh
python extract.py '/path/to/RB9.01.062.000 Gripper.STEP' /new/output/directory
```

The script refuses existing output directories, wrong source size/hash, optimized
Python, nested definitions, invalid or multiple solids, face count changes, lost
colors, and changed readback bounds or volume. It parses the same captured bytes
whose hash was checked. It emits only four selected parts and observations;
serialization includes a stored date, so bytes may vary between runs even with
the same OCP build. Review reproduction differed only at byte offset 78
(11/06/2026 versus 11/07/2026); retained artifact hashes remain exact. OCP exposes its package version,
not a separate runtime kernel fingerprint. Production-worker tests must still
qualify these retained exact bytes and the supported assembly workflow.

The gear's STL pose was not uniquely inferred. The initial root-bounds-filtered
fit refused a 0.06830928474664688 mm extent mismatch against its 0.05 mm
filter. A separate all-vertex, 24-proper-axis-rotation local-frame check with
0.0001 mm distance threshold admitted two poses (worst distance
0.0000255863 mm), differing by a local half-turn. These one-way vertex checks
do not prove surface equivalence. The retained diagnostic reports
`gear-root-fit.json` and `gear-local-fit.json` preserve both the rejection and
ambiguity; they are not acceptance oracles.
