# FTC09 source cross-section audit

`audit.py` reads the pinned FTC09 STEP bytes through the existing bounded source
parser and uses only Python's standard library and exact `Fraction` arithmetic.
It does not read a CAD kernel, an imported body, a native model, or a stored
mass oracle. The report records the source and generator hashes. The source
and attribution are documented in the parent trace README and public CAD
manifest.

From the four traced planes it calculates exact separation **3.03784 mm** and
exact top-to-floor separation **1.27 mm**. It matches 29 two-edge circular
bounds by their source X/Z centers on the two outer planes, with 25 matching
radii and four larger upper radii. Eight cone surface records form four
same-center pairs at those four changed-radius locations. These observations
are source cross sections; they do not establish which surfaces connect, the
material side, containment, disjointness, or through-hole topology.

The measured STEP plane separation is 0.1196 in. The existing
[reference history notes](../../../reference-histories/README.md) record a drawing
nominal thickness of 0.1195 in (3.0353 mm). This audit reads the STEP only and does
not equate those values or establish compliance with the drawing nominal.

The report gives exact coefficients of π for the 29 lower circles' combined
area and their hypothetical full-thickness cylinders. These values assume
complete, disjoint circles and through cylinders; no overlap, material, or
continuity proof is implied. It separately reports the hypothetical excess
volume of four complete 45-degree countersink frusta over the small-radius
cylindrical cores. That expression assumes frustum height equals the radius
difference and uses an ideal 45-degree slope. The STEP angle unit has a stated
finite decimal conversion and is not replaced with an asserted exact π/180.
These are conditional intermediate expressions, not the part's mass or area.

Each circular pair is checked by the parent `circle_pairs.py`; the arc-locus
interpretation remains conditional on directed trim semantics. Bottom loop
`#2794` and top loop `#3127` also have source endpoints off the exact circular
locus. The checker records this rather than modifying the source; its
conditional projection flag is not a source-accuracy or kernel-repair claim.

Whole-solid mass remains unavailable: the 24-edge outer boundary, four
four-edge openings, 16-edge opening, both 20/12-edge pocket floors, cone trim
domains, and global containment/connectivity are not integrated or certified.
No numerical π approximation, overall surface area, error budget, transfer
accuracy, physical measurement uncertainty, or `1e-6` trial-tolerance
qualification is provided. The report explicitly returns null for whole-solid
mass and area.

Run in the existing Python virtual environment from the repository root:

```sh
python fixtures/reference-oracles/nist-ftc09-direct/analytic_mass/audit.py > /tmp/ftc09-analytic-audit.json
python -m unittest discover -s fixtures/reference-oracles/nist-ftc09-direct/analytic_mass -v
```
