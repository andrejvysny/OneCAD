# Prusa box native-history diagnostic

The source is `fixtures/public-cad/prusa/MINI-fsenzor-box.stp`, SHA-256
`c8b06ef5271166c367d27be13e9c8f93a7ae07707033e4d23bd1ed5997285cb6`.
Original Prusa MINI / MINI+, PRUSA RESEARCH, source revision
`853bc30c4b10190f1d669ed6d0a567e333c28f21`, GPL-3.0-only. Preserve the source
attribution and license in `fixtures/public-cad/prusa`.

`extract.py` reads the pinned STEP with `cadquery-ocp-novtk` 8.0.1.1 and writes
`surface-inventory.json`, `bottom-stock-recipe.json`, and
`enclosing-stock-recipe.json`. The inventory records
166 faces: 93 planes, 36 cylinders, 34 cones, one torus, one sphere and one
B-spline surface. These values are same-kernel construction evidence, not an
independent oracle. Run the script using the prepared OCP Python environment.

The recipe contains only the lower planar face's outer wire: 20 lines and five
circular arcs. Its area is 1128.7536166548325 mm². Extruding it 13 mm upward
creates partial stock, not the source box: a same-kernel source-minus-stock
Boolean leaves 135.2159784231169 mm³. The bottom footprint omits some source
material; source overhangs must be modeled separately. Existing cavities,
fastener features, conical transitions and all other detail remain absent.
A deliberately oversized 61 × 28 × 15 mm rectangular stock at (−40, −26, −10)
is an alternative native starting point. The same-kernel source-minus-enclosing-
stock volume is zero; this diagnostic does not prove containment in another
kernel. The overstock is intentional, with no source movement; exact native
cuts and additions are still required. A successful stock test is not full-box
reconstruction or source equivalence.

In particular, source face 95 is a B-spline surface, about 22.00269 mm², with
bounds approximately X[13.5, 15.50215], Y[−7.75807, 0.15518],
Z[−6.88434, 2.84784]. Its exact native feature decomposition is not yet proven.
The surface is non-rational cubic × cubic with 4 × 8 control points and seven
B-spline trim edges. Its U control rows are close to linear but are not exactly
linear in the STEP decimal data; no exact ruled or native Loft equivalence has
been established. The current solver lane explicitly refuses spline sketch
entities (`protocol/SCHEMA.md`, §7.4), so these trim curves cannot be supplied
directly as native sketch regions; another exact analytic construction is not
ruled out but remains unproved. A separate native Fillet, Loft or analytic construction may generate the surface;
none has yet been proven equivalent. Seven shared edges border six planes and an
oblique cone, so its source feature sequence is unresolved.

The torus has major radius 1.7235203291577 mm and minor radius 0.4 mm around an
X-axis; it borders an X-normal plane and a conical surface. A 0.4 mm native blend
or a revolved circular section is a hypothesis, not a verified feature history.
The sphere has radius 4 mm, centre approximately (−25.00000048, −17.60000024, 0)
and borders a Y-axis radius-4 cylinder and several intersecting cones and a
radius-1.1 cylinder. A revolved spherical section is analytically possible;
the source Boolean order and exact result remain unproved.
No replacement by sampled geometry or unexplained tolerance is authorized.

The independent upstream STL report is
`fixtures/reference-oracles/prusa-box-stl.json` in the integration checkout.
Its trial bounds are 0.15 mm and 2% volume; these are diagnostics, not an exact
BRep-accuracy standard. No independent comparison, native GUI, undo/redo, or
fresh-process reopen has been performed for this incomplete stock.
