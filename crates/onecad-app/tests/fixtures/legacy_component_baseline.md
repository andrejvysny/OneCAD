# Legacy component fixture provenance

`legacy_component_baseline.onecad` is a newly captured baseline fixture, not a
previously shipped user document. It was written by the unmodified production
writer from commit `e50194e75cb96dad30c1b792f1deffeaffd72c02` while running
`a_baked_component_survives_save_and_reopen_with_no_library`. The test authored
a generator screw, exported its BRep, and saved a document-source placement
with legacy `sha256:` revision and locally cached BRep. The fixture is held
unchanged after capture.

- Fixture SHA-256: `94747540e5e57ff58269165eeb06ff7ea9878404f1d5cfe292aefab2d2735239`.
- Baseline `component_ops.rs` SHA-256: `08f0a92010ffeed4b65e6c9520ed41cbdf5f36523d63e30964a4cb6c17d77cc8`.
- Capture-only test source SHA-256: `b224730193cfbe8136e9c3447ec523915e6771be32e61b2649357774fe4ac587`.
- Worker SHA-256: `6a8ab293c097ec885ed86504f718b219d775f1d2a14452954524a83682bf8c9a`.

Capture used a `git archive` extraction of the baseline. The only test edit
copied the saved temporary file immediately after `rt.save` to
`/private/tmp/onecad-legacy-component.onecad`. Command, with the listed worker:

```sh
ONECAD_REQUIRE_WORKER=1 ONECAD_WORKER_PATH=/Users/andrejvysny/workspace/OneCAD/worker/build/onecad-worker CARGO_TARGET_DIR=/private/tmp/onecad-legacy-fixture-target cargo test -p onecad-app --test component_ops a_baked_component_survives_save_and_reopen_with_no_library -- --nocapture
```

The regression reopens the pinned bytes without a library, verifies the legacy
revision and source kind, and checks regenerated solid volume against the
analytic M6 screw volume on a fresh worker.
