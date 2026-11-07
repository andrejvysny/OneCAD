# Public mixed-source assembly stress

`public_mixed_stress.rs` prepares ten distinct definitions with two authored,
nonoverlapping occurrences each on an authored 200 mm grid. The grounded first occurrence is the Prusa box.
The imported sources are the Prusa box, four Robonine analytic XBF leaves, and
NIST FTC09 STEP. The native definitions are the
source-derived Prusa cover and lever, the existing plate with hole, and the fully
constrained synthetic 80 x 50 x 6 mm plate. An imported definition
is not a native feature history, and these placements do not establish physical fit.

The separate NIST import compares BRep volume and area with upstream-authored
values using the existing preregistered `1e-6` relative trial. That trial is not
an independently qualified `1e-6` accuracy bound. Source and transfer uncertainty
and a second-reader crosscheck remain unresolved; see
`fixtures/reference-oracles/nist-ftc09.json`. The complete Framework STEP is deliberately outside this admitted stress set: it
has six placed source solids, and its uncertified assembly-face identities currently
refuse ambiguous rebinding. The failed first prototype and source audit are retained
as separate negative evidence; they do not qualify a Framework assembly.
Prusa comparisons use existing source-derived diagnostic tolerances, not proof of
whole-shape equivalence.

The earlier 1000 mm grid was refused by the unchanged renderer 1/1024 mm
occurrence-precision guard. The compact layout is a new authored test placement;
it does not change source geometry, the precision threshold or frozen oracles.
Native scene admission must run separately from these service checks.

The test counts visible occurrence triangles and edge segments and unique MESH1
payload bytes against existing limits. It does not construct a `SceneSnapshot`,
so its byte count excludes scene metadata and BVH allocations. Renderer, physical
input, and device behavior require separate qualification. Attribution and license
terms are in the corresponding `fixtures/public-cad` and
`fixtures/reference-derivatives/robonine/analytic` directories.

Run from the repository root with the verified staged worker:

```sh
ONECAD_REQUIRE_WORKER=1 ONECAD_WORKER_PATH="$PWD/worker/build/onecad-worker" \
  cargo test -p onecad-app --test public_mixed_stress -- --test-threads=1 --nocapture
cargo clippy -p onecad-app --test public_mixed_stress -- -D warnings
```

To retain the saved container and provenance, set
`ONECAD_PUBLIC_ASSEMBLY_CAPTURE_DIR` to a directory where the
`public-mixed-10x20` child does not yet exist. The separately labeled `public-mixed-10x20-input` directory contains the authored
initial ten-definition input before regeneration. The final saved artifact follows
the independent-copy edit and therefore has eleven definitions and twenty
occurrences. Capture precedes cold-reopen validation; retain the test log alongside
the artifact.

The admitted-source service lifecycle passed with the real worker: twenty bodies,
233,286 visible triangles, 68,576 visible edge segments and 4,401,300 unique mesh
payload bytes. Exact saved geometry and projection survived a separate process and
fresh worker, then another edit and Undo. This does not establish scene/BVH memory,
native GUI input, latest-platform acceptance, physical fit, or a qualified NIST
accuracy bound. The separate Framework topology-rebind refusal regression verifies that a failed
replay adopts no partial definition or mesh generation. The fourteen ambiguous
references were diagnosed in a separate captured trace, not by this generic error.
