# 0018 — iPadOS hosts the OCCT worker in-process behind the same supervision

- Status: Accepted direction for the iPadOS track. Delivery and qualification
  status belongs in Plane (ONECAD-270, IP01 ONECAD-448); this record does not
  establish gate completion.
- Date: 2026-10-22
- Supersedes: the "iPad/iOS dropped from scope" scope lines of ADR-0015 and
  ADR-0016, for the iPadOS target only. Both remain in force for macOS.

## Context

The Platform and SDK Page reactivated the iPad track on 2026-10-21: native
UIKit host, shared Rust controllers, the existing wgpu renderer through Metal
and OCCT/PlaneGCS inside the iPad process. iPadOS forbids child processes, so
the macOS sidecar cannot be used. ADR-0016 kept the C ABI v3 static library
and spike evidence for exactly this case and listed its unresolved findings.

## Decision

1. iPadOS links `onecad_worker_lib` statically and runs it through
   `onecad_worker_run_config` on a dedicated thread over a `socketpair`. The
   wire stays OCW1/MESH1; no per-operation Swift/C++ API is added.
2. `onecad-app` gains explicit `sidecar` (default) and `embedded-kernel`
   features and a `WorkerHost` seam. Handshake, hello verification, fencing,
   poison keys and plan gates stay shared; only launch, exit observation and
   termination are backend specific.
3. The embedded release identity is a generated manifest v2 (`kind:
   "embedded"`) bound to target, SDK, deployment target and OCCT build. The
   desktop sidecar manifest and its release checks are unchanged.
4. A wedged in-process worker is abandoned once and the app enters a terminal
   `relaunch-required` state. There is no restart loop and no unsafe thread
   termination.
5. A worker-side journal persists the current job/op before execution so a
   crash or kill attributes suspected interruptions on the next launch without
   permanently poisoning innocent work.
6. The iPad UI reuses the egui editor of `onecad-ui` through a custom host
   that does not run eframe (`desktop` feature off). UIKit owns events,
   keyboard, accessibility elements and files.

## Consequences

- An OCCT crash on iPad terminates the app; recovery relies on autosave and the
  journal. Physical jetsam/crash qualification is ONECAD-469.
- The in-process kernel changes the linked artifact; LGPL/relink obligations
  for iPad distribution are tracked by ONECAD-256 and gate ONECAD-469.
- Fingerprints differ between macOS and iPad builds. They key disposable caches
  and poison only; documents remain portable.
- The ADR-0016 findings (non-retriable busy paths, `SinkScope` outside `try`,
  journal coverage gaps) are fixed in the worker as part of IP03.
