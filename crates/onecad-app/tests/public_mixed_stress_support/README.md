# Public mixed-source assembly stress

`public_mixed_stress.rs` prepares ten distinct definitions with two authored,
nonoverlapping occurrences each. The grounded first occurrence is the Prusa box.
The imported sources are the Prusa box, four Robonine analytic XBF leaves, the
complete Framework STEP, and NIST FTC09 STEP. The native definitions are the
source-derived Prusa cover and lever and the existing constrained plate. The test
requires all four Framework solids with distinct body IDs. An imported definition
is not a native feature history, and these placements do not establish physical fit.

The separate NIST import compares BRep volume and area with upstream-authored
values using the existing preregistered `1e-6` relative trial. That trial is not
an independently qualified `1e-6` accuracy bound. Source and transfer uncertainty
and a second-reader crosscheck remain unresolved; see
`fixtures/reference-oracles/nist-ftc09.json`. The Framework measurement oracle
covers one selected solid and is not an oracle for the complete imported assembly.
Prusa comparisons use existing source-derived diagnostic tolerances, not proof of
whole-shape equivalence.

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
`public-mixed-10x20` child does not yet exist. Capture precedes cold-reopen
validation; retain the test log alongside the artifact.

Status: this test has not yet been run with the real worker. The source asserts
four Framework bodies, but actual admission, resource totals, and the complete
mixed lifecycle have not yet been established.
