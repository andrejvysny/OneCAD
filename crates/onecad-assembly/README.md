# Assembly solver

This crate contains the pure occurrence, rigid-transform, joint and solver model.
Document authority, worker lifecycle, persistence and native presentation belong
to their existing application owners.

## Reproducible solver measurements

Build and run the measurement executable separately so sampling does not retain
a Cargo build lock:

```sh
cargo build --release -p onecad-assembly --example measure_grounded_tree
./target/release/examples/measure_grounded_tree > solver-measurements.json
```

When using `CARGO_TARGET_DIR`, run the executable under that directory instead.
Record the source and executable hashes, Rust toolchain, OS/build, device,
power/thermal state, concurrent workload and exact command beside the output.

The harness uses 20 bodies and 19 joints. It measures a grounded rigid chain,
a chain with one free slider, the same slider driven with 18 rigid followers,
and a translated case with a 100 mm local joint-frame offset. Bodies have no CAD
geometry in this tier. Five warmups precede 100 measured solves per case. JSON
retains every duration and outcome plus nearest-rank p50, p95, p99 and maximum.
Timing includes only `solve`; request construction and result checks are excluded.

Every converged result is checked against the fixture's known translations,
rotations, grounding, remaining DOF, limits and drive. Position tolerance is
`1e-5` mm, rotation-axis tolerance is `1e-7`, and the solver's own residual must
also satisfy its unchanged default tolerance. Valid initial free configurations
must remain unchanged. Refusal outcomes remain visible in the JSON rather than
being omitted from the sample set.

These measurements do not establish document-service, worker, renderer, input
or end-to-end native latency. They do not ratify or enforce an 8 ms target.
