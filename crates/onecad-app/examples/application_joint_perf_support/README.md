# Application joint publication timing

Use the checked-in [compact public fixture](../../../../fixtures/qualification/assembly-public-10x20/README.md). The fixture SHA-256 is checked before and after the run. Use a new results path. In a debug build, `ONECAD_WORKER_PATH` selects a compatible worker:

```sh
ONECAD_WORKER_PATH=/path/to/onecad-worker cargo run -p onecad-app --example application_joint_perf -- /path/to/native-compact-10x20.onecad /new/path/results.json
```

Release builds ignore `ONECAD_WORKER_PATH`. Before building, stage the matching worker integrity manifest in `packaging/binaries/onecad-worker-manifest.json`; the application embeds it at build time. Build with `cargo build --release -p onecad-app --example application_joint_perf`, then put the corresponding worker and its required kernel libraries beside the example executable, naming the executable `onecad-worker` on macOS or Linux (`onecad-worker.exe` on Windows). Run the example executable directly with the fixture and output paths. The normal release compatibility and integrity checks remain in force. The result records the resolved worker path, candidate SHA-256 before and after the run, and the adopted worker's handshake; a missing or changed candidate fails the run.

This headless application workflow authors one slider and 18 rigid joints with unbound logical frames derived from the original occurrence poses. Slider limits are symbolic, not physical travel limits. Before timing, all ten component definitions replay through the application and every published component mesh is obtained by scoped occurrence and generation. Five warmups precede 100 recorded alternating drives. Each timed await covers the generation-fenced application request through authoritative joint publication; pose and DOF checks run outside the timed interval. Warmups and samples are saved separately and incrementally, with pose errors, fixed validation goals, and request outcomes. The global translation and angular goals are benchmark criteria, not solver contracts. Invalid travel and pre-cancelled requests must leave the projection unchanged.

The measurement does not include GUI or presentation latency and does not establish contact, physical fit, travel, or a performance budget. Use an isolated host and record the host environment separately.
