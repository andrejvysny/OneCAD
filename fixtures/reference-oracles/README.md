# Public reference oracles

These records retain evidence from pinned public inputs. They are not generated
from OneCAD output. Keep imported-geometry checks distinct from native authored
feature histories and from a second-reader or second-kernel crosscheck.

`nist-ftc09.json` traces two identical upstream area/volume properties to the
same sole solid in the admitted NIST STEP. Inch units are explicit. The declared
source connectivity uncertainty is about 0.03381637 mm; it does not independently
justify a particular relative mass-property tolerance. The recorded `1e-6`
relative threshold was frozen as a trial before running OneCAD and must not be
widened after observing a failure. Independent numerical justification remains
part of qualification.

Run the production-worker check with the repository's staged pinned worker:

```sh
ONECAD_REQUIRE_WORKER=1 ONECAD_WORKER_PATH="$PWD/worker/build/onecad-worker" \
  cargo test -p onecad-app --test public_reference_inputs -- --nocapture
```

The test removes its temporary STEP after saving, then opens on a fresh worker
and checks identity and exact reproduction of the measured mass properties.
Its history remains `ImportStep`. To admit a new container seed after these
checks pass, set `ONECAD_REFERENCE_CONTAINER_OUT` to a new artifact path. Capture
uses exclusive creation and will not overwrite an existing fixture. Record its
hash, producing source, worker/kernel/toolchain and original STEP digest when
admitting it. Never recapture an existing failing fixture to obtain green tests.

The Framework PDF under `public-cad` describes a PCB plate. It must not be used
as a whole-enclosure dimension or mass oracle for the larger STEP assembly.
