# iPad performance evidence

Performance qualification uses the physical device and a pinned optimized app.
Simulator timings do not qualify the Air 4 GPU, touch delivery, presentation,
memory pressure or thermals. The functional simulator gate remains separate.

## Capturing

Launch the app with `ONECAD_PERF_TRACE=1` in its process environment. This enables
numeric Rust timing records and UIKit `os_signpost` intervals in subsystem
`com.onecad.ipad`, category `performance`. Without that exact value, no performance
writer starts, no performance directory is created and disabled spans do not read
a clock. This is available in Release and does not enable acceptance hooks.

The iOS host supplies its own `Documents/performance` directory to the shared
recorder. It starts one process-local background writer, which drains every
500 ms. Producers use a nonblocking lock and never write files or construct JSON.
The producer and writer batch each hold at most 16,384 fixed numeric records;
their combined record storage is bounded at 32,768 records. A full or contended
producer buffer drops timing evidence, never input. Counters expose these losses.

Each session produces `session-<pid>-<unix-ns>.json` metadata and numbered
`trace-<session>-<batch>.csv` files. Files first use a `.partial` suffix and become
`.csv` only after their complete write succeeds. Do not analyze `.partial` files
as completed batches. No raw document text, filenames or geometry payloads enter
timing records. Copy only this performance directory when collecting reports.

`onecad_ios_performance_status` returns current buffered, in-flight, dropped,
exported and export-error counters and requests an asynchronous flush. It is not
a disk-completion acknowledgement. `buffered=UINT64_MAX` means the buffer was
busy during the nonblocking query. These independently sampled counters may race
with production/export. The host requests this flush on scene resignation and
logs the counters; normal periodic export remains active. Failed batch writes
increase both dropped and export-error counts; their warnings are limited to
one per minute. Initial metadata or writer-start failures are also reported.

Keep an external companion record with the source revision and dirty patch,
app/worker SHA-256, UUIDs, configuration and build commands, device model/OS,
viewport size/scale, scene fixture identity, display mode, thermal/power state
and test procedure. Session metadata records the package version and Rust debug
assertion setting; it does **not** claim an embedded source or artifact hash.

## Numeric schema 1

Every data row contains:

```text
sequence,timestamp_ns,duration_ns,stage,correlation,a,b,c
```

`timestamp_ns` is the Rust recorder's session-relative monotonic arrival time.
A span is recorded at its **end**; subtract `duration_ns` to recover its start.
Instant events have zero duration. Sequence numbers order accepted records;
loss and export-error counters appear in each batch's comment header.

UIKit span durations and sample ages are computed using `CACurrentMediaTime` in
the host clock domain. Display-link values preserve that domain explicitly:
never subtract them from the Rust `Instant` epoch. Successful iOS host frames
and Rust frames use zero-based frame correlation IDs. Failed frames can diverge;
use timing and error/status evidence rather than assuming correlation equality
after a rejected frame. Shared viewport stages use the current egui frame ID;
post-`run_ui` paint stages compensate for its completed-frame increment.

| Stage | Code | Correlation | `a`, `b`, `c` |
|---|---:|---|---|
| TouchArrival | 1 | UIKit callback sequence | Oldest supplied touch age in ns, touch count, touch-type bitmask |
| DisplayLink | 2 | Host frame | Callback `CACurrentMediaTime` ns, link timestamp ns, target timestamp ns |
| HostFrame | 3 | Host frame | Reserved zeros; duration includes Rust frame and native output reconciliation |
| EditorFrame | 4 | egui frame | Reserved zeros; shared editor CPU span |
| Tessellate | 5 | egui frame | Reserved zeros |
| Acquire | 6 | egui frame | Reserved zeros; synchronous drawable acquisition CPU span |
| UiSubmit | 7 | egui frame | egui primitive count, zero, zero; queue submission CPU span |
| PresentRequested | 8 | egui frame | Suboptimal drawable flag, zero, zero |
| Accessibility | 9 | Host frame | Cumulative Rust snapshot reads, cumulative native rebuilds, Rust frame flags |
| TextInput | 10 | Host frame | Reserved zeros; native text snapshot/reconciliation CPU span |
| GpuComplete | 11 | egui frame | Reserved zeros; callback for work submitted through the UI queue submission |
| InputDrain | 12 | egui frame | Drained event count, deferred-frame flag, zero |
| SnapshotCache | 13 | Accessibility generation | Cache hit flag, flattened node count, zero |
| RustFrame | 14 | egui frame | Surface width px, height px, zero; total Rust host frame span |
| CameraApplied | 20 | egui frame | Captured view ID, desired camera generation, scene ID |
| CadSkipped | 21 | egui frame | Skip reason, captured view ID, desired camera generation |
| CadEncode | 22 | egui frame | Captured view ID, desired camera generation, scene ID |
| CadSubmit | 23 | egui frame | Captured view ID, desired camera generation, scene ID |
| HoverPick | 24 | egui frame | Captured view ID, desired camera generation, scene ID |
| CadFenceRegistered | 25 | egui frame | Sampling fence registered flag, CAD fence registered flag, zero |
| GpuSamplingComplete | 26 | egui frame | Reserved zeros; completion callback of the UI sampling fence |
| GpuCadComplete | 27 | egui frame | Reserved zeros; completion callback of the CAD lifetime fence |
| CadUniforms | 28 | egui frame | Captured view ID, uniform writes, uniform upload bytes |
| PreviewDesired | 30 | Scene session | Desired, submitted, displayed request sequences |
| PreviewSubmitted | 31 | Scene session | Request sequence, scene ID, gesture ID |
| PreviewCompleted | 32 | Scene session | Desired, displayed, validated sequences; reducer terminal receipt, including rejection |
| PreviewObsolete | 33 | Scene session | Request sequence, scene ID, gesture ID |
| PreviewService | 34 | Scene session | Request sequence, scene ID, gesture ID; total service/preparation span |
| PreviewQueued | 35 | Scene session | Request sequence, scene ID, gesture ID; residence ends on start, replacement or cancellation |
| PreviewKernel | 36 | Scene session | Request sequence, scene ID, gesture ID; API await only |
| PreviewScene | 37 | Scene session | Request sequence, scene ID, gesture ID; checked mesh decoding and scene/BVH construction |
| PreviewPacking | 38 | Scene session | Request sequence, scene ID, gesture ID; CPU prepared-scene packing |
| PreviewUpload | 39 | Scene session | Request sequence, scene ID, gesture ID; renderer replacement/suppression CPU call |
| PreviewDisplayed | 40 | Scene session | Request sequence, scene ID, gesture ID; renderer-accepted/displayable revision |
| PreviewDrawSubmitted | 41 | Scene session | Preview generation, egui frame, scene ID; CAD draw queued |

CAD skip reasons are 1 renderer busy, 2 sampling busy, 3 scene pending,
4 update pending, 5 clean and 6 resize refused. A captured view ID can remain
unchanged while desired camera generations advance; that distinction identifies
reuse of an older scene image. PreviewQueued is not necessarily followed by
PreviewSubmitted when a request was replaced or cancelled. PreviewService spans
overlap their child phases: do not sum overlapping spans.

## Presentation and frame scheduling

`PresentRequested` and `ONECAD_IOS_FRAME_PRESENTED` record a present request,
not its actual display time. GPU callback delivery is also not a hardware GPU
timestamp or proof of physical presentation. `PreviewDisplayed` records renderer
acceptance; `PreviewDrawSubmitted` records its CAD draw queued on the GPU queue.
Neither proves presentation. Use Instruments and device observation to establish
actual presentation, drawable stalls and touch-to-display latency.

The host requests a 60 Hz display-link range and remains on demand. Short retries
within one refresh interval reuse the next refresh; longer timers use the common
run-loop modes. Suspension preserves pending work without submitting background
Metal work. Input or a GPU-completion wake can arm another refresh during a frame.

The iOS host acquires its drawable after frame admission and editor validation,
before the shared editor checks CAD readiness. Air 4 investigation found that
late acquisition could block after a sampling-busy decision; the following UI
submission then replaced the sampling fence before the next readiness check.
Acquiring first moves that existing backpressure ahead of the check. The owned
drawable passes through the frame without holding a surface borrow across the
editor. An unavailable drawable still processes input/editor output and flushes
texture uploads; retry flags, suboptimal reconfiguration and both lifetime fences
remain unchanged. No GPU wait or queue-bound change is added. This ordering change
requires a new attributable device capture: confirm Acquire ends before
EditorFrame starts and measure ready-preview delays and sampling-busy runs.
It does not itself establish presentation latency or starvation elimination.

After the iOS host submits the UI pass, it immediately calls
`Editor::after_host_submit`. This registers both CAD and UI sampling lifetime
fences once. The desktop input-hook fallback remains; cleared pending flags make
repeated post-submit/fallback calls idempotent. Neither fence waits on UIKit.
Do not remove these fences or lower queue bounds to make a timing trace green.

## Repeatable validation

Pin a Release build before timing. Debug and Acceptance use the dev Rust profile
except the existing `naga` optimization override; their functional success is not
Release performance qualification. Separate first-use pipeline/font costs from
warm navigation. On Air 4, repeat the same one-finger orbit path on an empty
document and one simple saved solid, with fixed orientation, viewport and mode.
Keep the path inside unobscured canvas; separately test crossing chrome.

Measure callback/sample age, frame CPU percentiles, acquisition stalls, camera
generation versus submitted capture, CAD skip reasons, AX reads/rebuilds, uniform
upload counts, callback cadence and consecutive stale-image intervals. Confirm
idle/background behavior and no timing-evidence loss before interpreting results.
Record power/thermal conditions and run duration. Compare instrumentation enabled
and disabled to expose measurement overhead. The renderer's existing NEMA warm
completion test includes CPU encode/submission and GPU completion, not UIKit or
physical presentation.

Simulator validation can prove queue bounds, timing record schema, cache/focus
transitions, scheduler/fence invariants and deterministic camera trajectories.
It cannot establish Air 4 sustained performance or validate a quality tradeoff.
Keep the full functional simulator campaign and any supplemental diagnostics
attributable to their own source/product pins.
