# Mixed native Prusa assembly regression

`prusa_mixed_assembly` builds the cover from 32 native features and the lever from
8 native features, then embeds them beside the pinned public STEP box. The box
remains imported geometry. The initial workload has three definitions and four
occurrences, including a repeated lever. Making one lever independent creates a
fourth definition. This fixture does not itself establish the separate
10-definition/20-occurrence stress gate.

Placements are authored test inputs. The lever's candidate cylinder axis is
aligned to the box's candidate axis; axis-line checks admit antiparallel directions
and axial origin shifts, but reject perpendicular shifts, tilted axes and non-unit
directions under unchanged tolerances. These checks do not prove physical fit,
contact or rack-and-pinion coupling. Independent source-STL observations retain
their declared diagnostic bounds and limitations.

The production service workflow checks occurrence visibility and movement,
shared-definition edit, exact Undo/Redo mesh bytes, independent-copy isolation,
explicit ambiguous-topology refusal and recovery, and a positive 1 mm native body
translation. A counting admission gate denies the transient checkpoint reservation
once, requires unchanged projection and stale-mesh refusal, then allows an
immediate retry of the same pending edit. All temporary reservations must end;
the unchanged host cap is 805306368 bytes. The refused 4-to-3 mm cut remains a
negative case and is not silently rebound.

Manual Save and clean shutdown precede a separate OS-process/fresh-worker reopen.
The child replays all definitions, performs another translation and undoes it.
Native GUI input, attachment repair, physical input and source-equivalent fit need
separate evidence. General component repair remains open.

Run with the pinned production worker and an unused capture directory:

```sh
ONECAD_REQUIRE_WORKER=1 \
ONECAD_WORKER_PATH="$PWD/worker/build/onecad-worker" \
ONECAD_SOURCE_COMMIT="$(git rev-parse HEAD)" \
ONECAD_PUBLIC_ASSEMBLY_CAPTURE_DIR=/private/tmp/onecad-public-fixtures \
cargo test -p onecad-app --test prusa_mixed_assembly -- --nocapture
```

The optional capture refuses an existing labeled directory. Its manifest pins the
saved package, top-level test source, executable and resolved worker path bytes.
It labels native and imported definitions separately and records that cold-reopen
validation continues after capture. A capture alone is not a successful test.
