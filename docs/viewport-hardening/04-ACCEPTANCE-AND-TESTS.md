# Acceptance, regression fixtures, and release qualification

**Program:** OneCAD VP-HARDENING 1.0  
**Status of every application test in this document:** NOT RUN by the specification author. These are required implementation gates, not test results.  
**Normative companions:** [Specification](01-SPECIFICATION.md) and [numerical contracts](03-NUMERICS-AND-PROTOCOL.md).

## 1. Evidence levels and test architecture

A passing unit test proves only the behavior actually exercised. Maintain these separate lanes:

| Lane | Execution | What it can establish | What it cannot establish alone |
|---|---|---|---|
| U | Vitest/pure TypeScript with real math and owned resources | Camera equations, state transitions, validation, identity mapping, cache accounting | Actual GPU pixels or native input |
| W | CTest with the pinned real OCCT worker | Native curve/surface geometry, normals, protocol encoding, mesher failure handling | Frontend rendering or Tauri integration |
| X | Cross-track C++/Rust/TypeScript fixtures | Identical byte meaning, negotiation, version rejection, round-trip identity | Real user interaction |
| G | Real browser/WebGL diagnostic lane, including native renderer callbacks | Raster output, line width, render-order effects, renderer resources, browser recovery | Native worker geometry when using a mock backend |
| N | Packaged Tauri app with real IPC, worker, and target webview | End-to-end geometry, display, operation targeting, save/open, native integration | Physical input devices not actually used |
| P | Physical mouse/trackpad/touch/pen on declared hardware | Real device routing, gesture cancellation, display migration, wake/context behavior | Other untested devices or operating systems |
| B | Instrumented release-build benchmark on declared hardware | Measured throughput, latency, memory/admission behavior | Universal performance on unspecified systems |

Most important cases have two or more lanes. Do not replace W/N with mock geometry for a native tessellation claim. The existing Playwright mock lane DOES use real browser WebGL; retain its value, while labeling the backend correctly. [Baseline: REP-01 in the sources document.]

Name new test files after the existing repository conventions. Preserve the stable `TEST-*` identifier in the test title or metadata. Implement a verifier that checks every required ID has a test implementation and a recorded execution state. A discovered test is not the same as a passing test.

## 2. Required fixture catalog

Fixtures must be deterministic, generated from repository-owned definitions, and saved only into isolated QA locations. Every native fixture records OCCT build fingerprint, generator version, units, expected body/face identities, and requested display quality. Do not use screenshots as the sole geometry oracle.

| Fixture | Definition | Main purpose |
|---|---|---|
| F01 — line laboratory | Horizontal, vertical, and 15°/45° lines at several subpixel offsets; widths from VP03; dark/light flat backgrounds | CSS/device units, antialiasing, picking radius |
| F02 — unequal faces | Three faces with triangle counts 2, 7, 1; distinct authored colors; variant with a zero-triangle face between populated faces | Face ordinal versus triangle index; R18 |
| F03 — thin occluder | Two separate planar surfaces 1 mm apart, rear edge inside a 6 CSS-pixel acquisition circle; repeat with 0.01 mm and 0.1 mm gaps | Matched-ray visibility, no pointer-radius depth allowance |
| F04 — grazing neighbors | Visible boundary offset 3 CSS pixels from the pointer; another front surface intersects the pointer ray but not the boundary's matched ray | Prevent over-rejecting a visible nearby edge |
| F05 — pathological S-curve | Degree-five Bézier poles from NUM §2.6, plus its OCCT edge | Midpoint heuristic counterexample and independent error checking |
| F06 — weighted conics | Exact circular arc, ellipse, full periodic circle, positive rational Bézier with unequal weights | Rational subdivision, endpoints, seams, perspective projection |
| F07 — difficult splines | Nonuniform knots, repeated knots, inflection, very short span, C0 join, stationary tangent, tightly clustered knot intervals | Continuity splitting, angular-undefined handling, limits |
| F08 — shading gallery | Radius-20 cylinder, radius-20 sphere, torus R=30/r=8, box with tangent fillets, trimmed B-spline patch | Analytic normals, periodic seams, surface-dependent fallback |
| F09 — transforms | F08 under translation, rotation, orientation reversal, and a representation that retains an actual reflected transform | Normal transform and winding parity; avoid double reflection |
| F10 — offset precision | 10 mm part with 0.01 mm gap, translated to 10^6, 10^7, and 10^9 mm; separate 10,000 mm extent case | Quantization versus document magnitude |
| F11 — oblique sketch | Plane origin (100, 200, 300); normal proportional to (1, 2, 3); orthonormal X axis with a nonzero Z component | Full-basis alignment, screen metrics, restore |
| F12 — mixed sketch | Lines, circles, ellipses, arcs, construction curves, nested closed loops; one capped very large curve and small curves | Per-entity refinement and fill/stroke agreement |
| F13 — dense sketch | Deterministic 5,000-entity fixture; 20,000-entity stress variant; fixed seed and diverse constraint styles | Stable geometry, batches, hover, bounded upload |
| F14 — section solids | Box, hollow bushing, nested cavity, two overlapping closed bodies, compound with several solids, open shell, non-manifold sample | Per-solid caps, holes, invalid cap suppression |
| F15 — replacement preview | Boolean cut/add/new-body workflows with an exact candidate mesh and explicit replacement IDs | Effective display set and preview clipping/caps |
| F16 — assembly | 100 bodies totaling 1M triangles and 200k segments; 1,000-body and 5M-triangle stress variants kept separate | Main benchmark and many-object overhead |
| F17 — one large body | One body near 1M triangles, with known face ranges and mixed colors | BVH build, color memory, highlight allocation |
| F18 — adversarial bytes | Structurally valid payload mutations for every semantic invariant and size boundary | Validation and admission controls |
| F19 — publication races | Controllable fake transport/preparation completion order plus real native operation sequence | Stale results, close/reopen, quality jobs |
| F20 — lifecycle | Context loss, dispose during init, remount, repeated document and preview cycles | Recovery and resource retirement |

Use small purpose-built fixtures for correctness. Do not require a million-triangle assembly to reproduce a two-face identity bug. Store fixed camera states separately from the model geometry so screenshots are reproducible without constraining model IDs.

## 3. Measurement methods and independent oracles

### 3.1 Pixel widths and edge visibility

F01 renders into a real WebGL context with the actual installed `LineSegments2.onBeforeRender`. Capture only after the tested submission. Measure in the drawing buffer, then divide by the actual renderer DPR to express CSS pixels.

Measure coverage-equivalent width from integrated contrast across several perpendicular scan lines away from caps and joins. Measure raster footprint separately. Do not expect antialiased edge pixels to equal a material token exactly. Compare repeated subpixel positions, not one lucky aligned line. Required mean coverage-width error is at most **0.25 CSS pixels**; cross-DPR mean difference at most **0.20 CSS pixels** for the same authored width. Record shader, framebuffer format, MSAA samples, and background. A diagonal's perpendicular scan must be geometrically correct.

Picking tests invoke the normal input path and inspect the resulting typed reference and mesh publication proof. A CPU helper returning a plausible edge is not sufficient if a real click selects something else. Native screenshots show which boundary was visible; reference fixtures provide the expected occlusion independently of the production picker.

### 3.2 Curve error and normals

For known circles/ellipses/Bézier curves, compute expected points from independent analytic equations or an independent evaluator, not by calling the production polyline builder. Check both the implementation's conservative bound and a dense/adaptive sample of the original curve. Samples are a regression oracle, not proof that an arbitrary continuous curve has a certified global bound.

For each curve segment, compare its finite-segment distance, not distance to an infinite line. Test the reported certificate against the control hull. Positive homogeneous weights and clip-space W conditions must be checked. The approximate OCCT fallback is labeled estimated, even when sample tests pass.

For normals, compare unit vectors with `acos(clamp(dot,-1,1))`; handle sign separately using outward orientation. Smooth analytic fixtures require **≤0.1° normal error**, and matched normals at an analytically tangent seam **≤0.2° difference**, away from genuinely undefined singularities. Singular fixtures assert the specified fallback/split status rather than manufacturing one exact normal. Grazing-light screenshots complement, not replace, vector checks.

### 3.3 Lifetime and upload accounting

Warm the scene, then record renderer geometry/texture/program counters and application-owned allocations. Count typed-array bytes, buffer creation/disposal, upload ranges, registry leases, and cache entries. Renderer counters do not directly equal measured VRAM. Label estimated GPU bytes separately from system GPU-memory measurements.

During 1,000 alternating rendered hovers, sample after each block of 100. After the first 200, no unexplained positive per-cycle growth is allowed. After caches are cleared and resources retired, application resources return exactly to the expected baseline; renderer-internal reusable resources may remain only at a documented stable baseline. Repeat the full cycle three times. A total scene-child count is not an ownership test.

### 3.4 Performance run protocol

Use a release build on the named reference machine, plugged in with power/thermal conditions recorded. Disable verbose stack capture and test-only GL interceptors for timing runs. Keep resource instrumentation light; perform separate heavy diagnostic runs.

Each workload gets a 5-second warm-up and five 10-second deterministic interaction runs: orbit, pan/zoom, hover sweep, selection changes, and section manipulation. Report per-run p50/p95/p99/max, aggregate distributions, draw calls, uploaded bytes, CPU stage durations, and quality state. Do not average percentiles. Repeat cold shader/build behavior separately and preserve outliers with traces.

Exclude genuine settled idle gaps from active-frame distributions; do not exclude slow active frames. Pointer-to-submission and actual presentation/physical observation are different metrics. Label them accurately. Fixed replay input must not coalesce away most of the intended workload unnoticed.

## 4. Renderer, line, and lifecycle cases

| ID | Procedure | Required result | Lanes |
|---|---|---|---|
| TEST-LINE-01 | Render F01 at DPR 1/1.5/2 with all baseline widths, both themes and projections. | Coverage widths satisfy §3.1; resolution is logical-pixel size at actual draw time. | U,G,N |
| TEST-LINE-02 | Move a stationary native window between DPR-1 and DPR-2 displays without CSS resize; simulate the same event in browser. | One event-driven resize/redraw; widths remain consistent; no polling loop and no stale body/highlight/static-sketch constants. | U,G,P |
| TEST-LINE-03 | Probe just inside/outside a 6 CSS-pixel edge radius at each DPR before first render and after render. | Same acquisition boundary within numerical test tolerance; drawing width does not silently alter intended pick radius. | U,G |
| TEST-LINE-04 | Render active/static/draft/construction/dimension/selection lines side by side, resize, theme flip, and zoom. | All use their declared CSS widths; dashes follow NUM §3.5; point textures keep their separate device-pixel contract. | G,N |
| TEST-LIFE-01 | A contribution invalidates once during its frame hook. Repeat from a synchronous after-render callback. | Exactly the necessary subsequent frame executes, then the engine becomes idle; no dirty state is lost. | U,G |
| TEST-LIFE-02 | Dispose before asynchronous renderer initialization resolves; mount/unmount twice as StrictMode does. | Renderer/context and listeners are released exactly once; no callback accesses disposed state. | U,G |
| TEST-LIFE-03 | Lose and restore a live WebGL context with bodies, environment, highlights, and section active. | Clear unavailable/recovering status; CPU state retained; restored scene and PMREM are rebuilt after renderer recovery; no busy retry. | G,N |
| TEST-LIFE-04 | After all publications, tweens, observers, and one-shot refinement settle, observe for 2 seconds. | Zero application rAF requests, renders, mesh jobs, and recurring polling timers. A later display change still wakes rendering. | U,G,N |
| TEST-LIFE-05 | Renderer submission throws or an experimental async submission rejects; a completion listener is registered. | No successful publication acknowledgment for the failed submission; diagnostics and recovery/disposal remain usable. | U,G |
| TEST-BACKEND-01 | Open production settings with an old experimentalWebGpu=true preference. | Supported WebGL path selected, preference migration explained; no silent missing line/cap backend. | U,N |
| TEST-BACKEND-02 | Exercise stencil/MSAA/clip-control capability combinations. | Required context properties checked; ordinary depth fallback passes identical semantic tests; unsupported context gives actionable failure. | G,N |
| TEST-BACKEND-03 | Run initialization failure followed by safe retry/new renderer. | No reused incompatible canvas context, listener leak, duplicate engine, or renderer success claim before feature checks. | U,G |

## 5. Ownership, mesh validation, and publication cases

| ID | Procedure | Required result | Lanes |
|---|---|---|---|
| TEST-RES-01 | F17: 1,000 alternating face hovers with actual renders, then clear cache. | Counts plateau per §3.3; body remains drawable and pickable after owned highlight disposal. | U,G,N |
| TEST-RES-02 | Hold whole-body selection during mesh replacement and close. | Selection borrows the exact geometry object through a lease; old resource retires only after all holders release it. | U,G |
| TEST-RES-03 | Select/deselect many faces and edges until the overlay budget is reached. | Cache ≤64 MiB and ≤256 entries; no silent eviction of live semantic selection; explicit bounded degraded visualization where prescribed. | U,G,B |
| TEST-RES-04 | Dense sketch hover, marker changes, draft updates, and growth beyond current buffer capacity. | Unchanged committed buffers retain identity; replaced buffers are retired; allocation/upload is bounded and measured. | U,G,B |
| TEST-RES-05 | Fifty isolated document open/close cycles plus 100 preview apply/cancel cycles. | Leases, detached jobs, observers, and application allocations return to baseline; renderer internals plateau. | U,N |
| TEST-MESH-01 | Mutate magic/version/flags/section overlap/alignment/duplicate types/reserved fields/truncation. | Typed rejection before construction; allowed unknown optional sections handled by the correct version contract. | U,X |
| TEST-MESH-02 | Inject NaN/Infinity, bad bounding boxes, invalid normals, and triangle index ≥V. | No geometry installation or unbounded allocation; precise diagnostic identifies invariant and body/publication. | U,X |
| TEST-MESH-03 | Mutate face ranges, edge ranges, zero-length ranges, ID offsets, invalid UTF-8, duplicate IDs. | Valid zero entries accepted where specified; impossible ownership or ID tables rejected without wrong face mapping. | U,X |
| TEST-MESH-04 | Counts near integer overflow and memory caps, including legal-small blob with huge implied edge expansion. | Checked arithmetic and admission reject before derived allocation; limits apply across concurrent jobs. | U,X |
| TEST-MESH-05 | Fail a replacement validation while the prior body is installed. | Last valid body remains inspection-only, with historical provenance; operation acquisition cannot promote it as current. | U,N |
| TEST-MESH-06 | Native mesher returns a missing nondegenerate face or unsamplable nondegenerate edge; include valid declared degenerate topology. | Missing required geometry cannot be published as complete; known degenerates receive explicit status, not fabricated +Z geometry. | W,N |
| TEST-PUB-01 | F19 returns old generation after a new generation, with both parsing and preparation jobs reordered. | Only exact current identity installs; old results release CPU/resources once and never affect selection. | U,N |
| TEST-PUB-02 | Close document A and open B reusing a body-local ID before A's job returns. | Session/document fences reject A; no shared-cache identity collision. | U,N |
| TEST-PUB-03 | Capture a pick, replace topology, then request semantic promotion using the captured proof. | Old proof is rejected or explicitly resolved through authoritative repair; no nearest-topology guessing. | U,N |
| TEST-PUB-04 | Change only display detail while keeping topology fixed; complete two quality jobs in reverse order. | Stable face/edge identity and selection survive; worse/old quality does not overwrite accepted requested quality. | U,W,N |

Use property-based semantic mutation tests with fixed failing seeds and a checked-in corpus. Start with 10,000 generated small payloads per CI run and a bounded nightly 100,000-case run. These are test counts, not a mandate to allocate huge payloads. Preserve every newly discovered minimal counterexample. Existing fuzz tooling may implement the generator; no dependency addition is required solely to use a specific testing framework.

## 5.1. Picking and hover currentness cases

| ID | Procedure | Required result | Lanes |
|---|---|---|---|
| TEST-PICK-01 | F03 with rear edge 1 mm behind front face, first at the legacy 260 mm/76°/1,000 CSS-pixel setup, then current defaults. | Ordinary selection rejects the hidden edge despite acquisition radius; previous world-depth bias counterexample cannot recur. | U,G,N |
| TEST-PICK-02 | F04 visible neighbor whose candidate ray differs from pointer ray, both camera projections. | A visible nearby boundary remains acquirable; matching the wrong pointer ray cannot over-reject it. | U,G,N |
| TEST-PICK-03 | Unequal endpoint depths and near-clipped edge in perspective; move cursor along projection. | Correct perspective parameter reconstructs the geometric anchor; no screen-linear parameter used as world-linear without correction. | U,G |
| TEST-PICK-04 | Thin gaps and adjacent faces at different tessellation levels, with certified versus estimated error metadata. | Numerical tolerance separate from approximation uncertainty; uncertain edge does not defeat visible face; refine/status path stays explicit. | U,W,N |
| TEST-PICK-05 | Explicit overlap cycling/pick-through over coincident or nested bodies with stable IDs. | Deterministic order and typed references; hidden candidates appear only under the explicit policy, not an ordinary depth-bias side effect. | U,N |
| TEST-PICK-06 | Keep pointer stationary; change camera, section, geometry, modifier, isolation, or display mode. | Hover recomputed from current projection/display revision; no stale highlighted target; result deduplicated only after current query. | U,G,N |
| TEST-PICK-07 | Capture hit then replace geometry, rebase origin, remove body, or change runtime session before a tool consumes it. | Identity proof checked at consumption; correct global anchor retained; invalid target fails closed without guessed repair. | U,N |

## 6. Appearance, normals, and color cases

| ID | Procedure | Required result | Lanes |
|---|---|---|---|
| TEST-APP-01 | Enter/exit sketch with plain, imported-colored, and multiple assembly-colored bodies, including sets created mid-session. | Derived state consistent for every set; opaque focus state and model restoration exactly match VP06. | U,G,N |
| TEST-APP-02 | Switch render mode, theme, and section while focused; repeat every order. | No nested save/restore collisions, stale clipping, or leftover opacity. | U,G,N |
| TEST-APP-03 | Toggle assembly colors on F02 and an authored-color assembly, then return to authored mode. | Diagnostic body colors override display only; original authored data and undo history unchanged. | U,N |
| TEST-APP-04 | Render imported color swatches and neutral surfaces under fixed studio light. | Input sRGB converted once to linear; output sRGB once; vertex colors not double-tinted; overlays not accidentally tone-mapped. | U,G |
| TEST-APP-05 | Closed solid, open sheet, and reversed/reflected fixtures under normal and section views. | Sidedness follows validated topology/normal status; no premature backface-culling loss. | W,G,N |
| TEST-COLOR-01 | F02 with triangle counts 2,7,1 and face-specific red/green/blue overrides. | Ordinal lookup uses idOf; each face receives only its own override. | U,G,N |
| TEST-COLOR-02 | Insert zero-triangle face; use indexed vertices shared across two face-color ownership domains. | Empty face does not shift identities; selective vertex split prevents color interpolation without changing triangle order. | U,X,G |
| TEST-COLOR-03 | Change one authored face color on F17 and toggle theme. | Unaffected buffers preserved; authored colors immutable through theme; fallback colors update; no unconditional full de-index/reload. | U,G,B |
| TEST-NORMAL-01 | Planar box faces, both orientations. | Unit normals align with exact outward directions; sharp edges remain sharp. | W,G |
| TEST-NORMAL-02 | Cylinder periodic seam and tangent fillet chain. | Smooth normals meet §3.2; topology remains split/addressable; no welded sharp features. | W,N |
| TEST-NORMAL-03 | Sphere poles, torus seam, cone apex. | Sphere radial normal valid; cone apex uses per-fan rule and diagnostic; no unconditional global +Z fallback. | W,N |
| TEST-NORMAL-04 | F09 actual transform and face-orientation combinations. | Normal transform/winding parity correct exactly once; triangle normals and front-facing material agree. | W,G,N |
| TEST-NORMAL-05 | Trimmed spline surface with irregular triangulation and singular derivative sample. | Regular nodes use surface normals; bounded fallback provenance present for exceptional nodes; false certainty rejected. | W,N |
| TEST-NORMAL-06 | Change tessellation level on F08 under a narrow grazing light. | Normals remain geometry-consistent; no density-dependent tangent seam; no discarded legitimate hard boundary. | W,G,N |

## 7. Curve and sketch fidelity cases

| ID | Procedure | Required result | Lanes |
|---|---|---|---|
| TEST-CURVE-01 | F05 at chord budgets 0.05,0.01,0.001 mm. | Never one incorrect straight segment; independent max error within requested bound plus reported quantization error. | U,W,N |
| TEST-CURVE-02 | Exact conics and positive weighted Béziers under reversal and trims. | Stable endpoints and monotonic parameter intervals; rational subdivision conserves original curve. | U,W |
| TEST-CURVE-03 | Repeated knots, C0 joins, and nonuniform spans from F07. | Splits occur at required boundaries; no tangent cone spans across a genuine discontinuity. | U,W |
| TEST-CURVE-04 | Long bulge with midpoint/chord coincidence; loop whose endpoints coincide. | Finite-chord control-hull criterion catches interior deviation; zero chord handled by subdivision/typed degeneracy. | U,W |
| TEST-CURVE-05 | Stationary derivative, cusp, and nearly vanishing derivative coefficients. | No NaN/angular false pass; angular undefined is reported; chord accuracy still enforced where representable. | U,W |
| TEST-CURVE-06 | Hit recursion/segment/work budget on pathological data. | Explicit quality-limited or failure result; no success certificate fabricated at cap; responsiveness/cancellation preserved. | W,N |
| TEST-CURVE-07 | Procedural curve routed through the fixed approximate OCCT fallback. | Original-curve independent check passes chosen tolerance; status remains estimated, never upgraded to certified by sampling. | W,N |
| TEST-SK-01 | Hover across F13 while instrumentation tracks all entity geometry. | Only style/hover overlay changes; unaffected committed geometry identity and uploaded geometry bytes unchanged. | U,G,B |
| TEST-SK-02 | Move one constrained point through authoritative solved states. | Dirty incident entities/batches update; unchanged entities stay stable; current solver authority preserved. | U,N |
| TEST-SK-03 | 1,000 draft line/arc and trim-ghost updates with buffer growth. | Reusable capacity and bounded allocations; no full scene rebuild; cancel releases transients. | U,G,B |
| TEST-SK-04 | Cross chunk boundaries at 4,096 segments, select mixed entities, then delete/undo. | Segment-to-entity identities and stable compaction map stay correct; no dangling slot pick/hover. | U,N |
| TEST-SK-05 | F12 large capped circle plus small circle during fourfold zoom. | Small entity refines independently even when maximum large-curve count is unchanged. | U,G,N |
| TEST-SK-06 | Render same curve in active/static/draft/trim channels, near-front and oblique perspective. | Shared .25/.35-pixel quality policy and statuses; exact projected hull used where valid; no legacy fixed-count path. | U,G,N |
| TEST-SK-07 | Nested circles, annulus, concave region, and incomplete live loop during retessellation. | Fill and stroke share sampling revision; authoritative holes preserved; provisional closure never becomes operation authority. | U,W,N |
| TEST-SK-08 | Curve crossing near plane or with projected hull behind eye; pan far from sketch origin. | No enormous/nonfinite tessellation; near subdivision/culling correct; no stale metric reuse; snap result independent of render count. | U,G,N |

A sketch test comparing two channels must use the same authoritative entity, not visually similar independently generated entities. Include fixed text annotations and selection styles because material-family differences often hide the remaining inconsistent path.

## 8. Camera and physical input cases

| ID | Procedure | Required result | Lanes |
|---|---|---|---|
| TEST-CAM-01 | Stationary orthographic circle; resize from square to 2:1 then 1:2, including safe-area panel changes. | Circle stays circular; projection and fit use the new measured dimensions without an extra orbit input. | U,G,N |
| TEST-CAM-02 | Repeat anchored zoom at both distance/scale limits, off-center pointer. | No target drift when effective factor is 1; projected anchor displacement ≤0.25 CSS pixels within qualified precision. | U,G,N |
| TEST-CAM-03 | Interrupt Home/Fit/ViewCube tween with wheel, pan, touch pinch, and cube drag. | Accepted manual input cancels tween; subsequent tick does not overwrite new state. | U,N,P |
| TEST-CAM-04 | Switch perspective/orthographic at several targets, scales, and FOVs. | Pivot-plane apparent scale preserved; migration of old views preserves existing appearance. | U,G,N |
| TEST-CAM-05 | Enter F11 arbitrary-plane sketch, edit, then exit/cancel. | Plane X projects right and Y up; orthographic exact plane view; full prior model quaternion/target/scale/projection restored. | U,N |
| TEST-CAM-06 | Fit tall/wide/offset bounds into measured safe rectangle with inspector open/closed. | All required corners inside padding; impossible fit returns explicit failure; no hidden assumption about symmetric viewport. | U,N |
| TEST-CAM-07 | Zoom near faces, pan behind bounds, and grow/shrink scene; run ordinary and reversed depth. | Adaptive clipping expands immediately and contracts with specified hysteresis; no spurious model clipping or depth-policy inversion. | U,G,N |
| TEST-CAM-08 | Pan by 100 CSS pixels at different FOVs/projections, snap poles, and edit FOV. | Exact scale formula produces intended motion; no roll discontinuity at pole transition; user view remains finite. | U,N |
| TEST-INPUT-01 | Mouse LMB tool/selection, RMB pan, Shift+RMB orbit, MMB pan, and UI chip interaction. | Existing mappings retained; UI/tool ownership established before capture; no unintended LMB orbit. | U,N,P |
| TEST-INPUT-02 | Armed tool versus actual drag, with every nav source. | Armed tool permits inspection; active drag freezes camera consistently; no solver dimension jump. | U,N,P |
| TEST-INPUT-03 | Synthetic mouse-wheel, trackpad-wheel, and WebKit gesture sequences with modifiers/source overlap. | Reducer emits one semantic nav action; no double pinch zoom or mid-gesture source flip. | U,N,P |
| TEST-INPUT-04 | Pointercancel, lostpointercapture, window blur, canvas removal, and tool cancel. | Ownership and pointer map released once; no stuck drag/nav state; next gesture begins cleanly. | U,G,N |
| TEST-INPUT-05 | Move two touches together by a known screen translation, then combine translation and changing separation. | Centroid delta counted once, pinch uses separation ratio, anchor follows gesture center without compounded pan. | U,P |
| TEST-INPUT-06 | One-finger navigation on empty model canvas; one-finger tool gesture; add/remove second finger. | Explicit owner policy applies; transition rebases start samples and does not jump or accidentally commit. | U,P |
| TEST-INPUT-07 | Pen draws while a second touch requests navigation; pen lifted/cancelled with lingering touch. | Active pen/tool drag freezes camera; no touch steals tool ownership; handoff is explicit and stable. | U,P |
| TEST-INPUT-08 | Physical native trackpad momentum, pinch, natural scroll, modifier changes, and window focus change. | No unintended page zoom, sticky modifier, post-cancel tween, or duplicate input; record actual device/platform. | N,P |

A P-lane case without hardware is `blocked-native`/`blocked-device`, not passed by a synthetic PointerEvent. Implement its automated reducer counterpart and keep the physical checklist open. This does not prevent completing independent work packages.

## 9. Precision, protocol, and detail scheduling cases

| ID | Procedure | Required result | Lanes |
|---|---|---|---|
| TEST-PROTO-01 | Decode canonical v1 byte fixtures with old world coordinates and version-2 fixtures with local origins. | No reinterpretation of v1; little-endian fields and both coordinate meanings exact. | U,W,X |
| TEST-PROTO-02 | Encode v2 in C++, validate/forward in Rust, decode in TypeScript. | All added sections, float64 alignment, flags, bounds, IDs, edge/face adjacency, and error statuses agree byte-for-byte. | X,N |
| TEST-PROTO-03 | Mismatch worker capabilities, mesh version, section availability, and client request. | Negotiated supported path or typed rejection; no silent origin-zero substitute for required v2 data. | X,N |
| TEST-PROTO-04 | Old cached v1 display mesh opened under new app and saved again. | User BRep/history unchanged; cache recognized as legacy and regenerated safely; no automatic destructive format rewrite. | X,N |
| TEST-PROTO-05 | Different quality levels, origins, kernel fingerprints, and topology generations use similar body IDs. | Typed cache keys cannot collide; no undocumented parsing extension of old key strings. | U,X,N |
| TEST-PROTO-06 | Reserved bits, invalid quality status, missing solid IDs, malformed adjacency, nonfinite origin, and non-outward bbox rounding. | Correct version-specific rejection; no capped rendering from uncertified solid membership. | U,X |
| TEST-PREC-01 | F10 at origin and translations up to 10^9 mm. | Body-local f32 positions equal the origin case within recorded quantization error; global/render conversion reconstructs correct placement. | U,W,N |
| TEST-PREC-02 | Render-origin rebase while a body, section, sketch, preview, lights, and HTML annotation are visible. | All move through one atomic coordinate mapping; no frame with mixed origins or anchor jump. | U,G,N |
| TEST-PREC-03 | Pick then promote a point before/after rebase; perform a real operation. | Backend receives authoritative global position and correct topology proof, not render-relative position. | U,N |
| TEST-PREC-04 | 0.01 mm gaps at increasing local extent/zoom, ordinary and reversed depth. | Qualified cases remain distinguishable; actual quantization/depth limits reported rather than claimed solved by camera motion. | W,G,N |
| TEST-PREC-05 | Extreme/out-of-envelope coordinates or camera request. | Bounded rejection/quality message; no NaN view, failed giant allocation, or false precision certificate. | U,N |
| TEST-LOD-01 | Orbit without meaningful scale change for 10 seconds. | No mesh job per orbit frame; stable compatible quality remains installed. | U,N,B |
| TEST-LOD-02 | Zoom through several refinement levels and immediately reverse direction. | Quantized requests, hysteresis, concurrency and latest-wins rules; bounded coalesced work. | U,N |
| TEST-LOD-03 | Edit model during quality job; close/reopen during second job. | Runtime/topology fences reject obsolete results; stable IDs survive detail-only swaps. | U,W,N |
| TEST-LOD-04 | Exceed triangle/segment/memory budget at high zoom. | Last valid approximation remains visibly marked limited; no unbounded work or false .35-pixel guarantee. | U,N,B |
| TEST-LOD-05 | Request display refinement then export. | Display settings do not weaken export tolerance or modify model history; cache and policy axes are separate. | W,N |

## 10. Edges, sections, and previews

| ID | Procedure | Required result | Lanes |
|---|---|---|---|
| TEST-EDGE-01 | Cylinder/sphere periodic seam, tangent fillets, open shell, hard corner. | Correct metadata classes and default visibility; visible hard/boundary/unknown retained; verified smooth seams subdued/hidden. | W,N |
| TEST-EDGE-02 | Ambiguous/poorly supported edge continuity, non-manifold adjacency. | Unknown status remains visible; no sample-only proof hides a possible crease. | W,N |
| TEST-EDGE-03 | Switch feature/all-topology/wireframe modes, then use explicit edge acquisition. | Hidden display classification does not delete geometry/IDs; tool mode can intentionally acquire applicable hidden topology. | U,N |
| TEST-EDGE-04 | Sphere/cylinder whose silhouette is not a BRep edge, both themes. | Smooth surface boundary readable without fake pickable silhouette IDs or triangulation-wireframe edges. | G,N |
| TEST-SECTION-01 | Plane through solid box and hollow bushing at several offsets/directions. | Correct filled cut material and cavity hole; original kept half and normals consistent. | W,G,N |
| TEST-SECTION-02 | Multiple overlapping closed solids and compound solids. | Per-solid stencil process avoids signed cancellation between independent solids; cap depth ordering stable. | G,N |
| TEST-SECTION-03 | Open shell/non-manifold/incomplete mesh included. | No invented solid cap; clipped sheet remains visible with explicit cap-ineligible status. | W,N |
| TEST-SECTION-04 | Enter/exit sketch while section active; switch mode/theme. | Opaque focus retains correct cap; active sketch stays readable under declared x-ray policy; no transparent-pass accident. | G,N |
| TEST-SECTION-05 | Select front cap pixel with real geometry behind, ordinary and pick-through modes. | Ordinary hit blocked by visualization cap; no fake editable face; explicit pick-through behavior labeled and deterministic. | U,N |
| TEST-SECTION-06 | Plane through world grid, close coplanar surfaces, and selected edge. | No depth writing by annotation ink; bias correct under ordinary/reversed depth; no persistent stippling. | G,N |
| TEST-SECTION-07 | Move section while body visibility, isolation, and geometry revision change. | Cap population uses one effective display snapshot; transforms/provenance current; no stale stencil resource. | U,N |
| TEST-PREVIEW-01 | F15 exact Boolean candidate replaces committed bodies while section is active. | Preview surfaces/edges/caps derive from effective display set; replaced committed geometry not double-drawn. | U,W,N |
| TEST-PREVIEW-02 | Cancel preview after user changes body visibility/isolation. | Latest user visibility restored, not a stale saved boolean; preview-owned resources released. | U,N |
| TEST-PREVIEW-03 | Superseded preview returns after another operation or document close. | Preview session/publication fence rejects late result and releases it; no overwritten model. | U,N |
| TEST-PREVIEW-04 | Approximate interactive preview followed by exact preview, then commit/failure. | Distinct quality/role state; no approximate preview becomes authoritative operation geometry or cap proof. | U,W,N |

Pixel and picking tests must cover the same cut plane and effective display set. In the cap CPU test, use simple known-solid point membership as the oracle; do not compare two calls to the production ray-parity routine. Add near-boundary and cavity cases where parity rays hit mesh vertices/edges and require the specified ambiguity handling.

## 11. Acceleration, performance, grid, and capture

| ID | Procedure | Required result | Lanes |
|---|---|---|---|
| TEST-BVH-01 | Compare baseline triangle enumeration with indirect BVH for fixed randomized rays on F17. | Same original triangle and face identity, point, sidedness, and nearest accepted hit. | U,G |
| TEST-BVH-02 | Nearest triangle clipped away but farther triangle remains; repeat in all-hit mode. | Acceleration does not stop at a clipped nearest hit; ordinary/overlap policies match reference. | U,N |
| TEST-BVH-03 | Build, serialize/transfer where used, and replace BVH after quality change. | No original index-buffer reorder; documented index resolver contract; stale build rejected. | U,N |
| TEST-BVH-04 | Segment/body bounds hierarchy with offscreen/hidden/classified edges. | Conservative acquisition culling, correct CSS-radius expansion; no visible-edge false negatives. | U,G |
| TEST-BVH-05 | Dispose/cancel preparation during BVH build or low-memory admission. | No detached backing buffer still in use; total reservations released; no stale installed tree. | U,N |
| TEST-PERF-01 | F16 main 1M/100/200k workload using §3.4. | p95 active interval ≤17.5 ms, p99 ≤33.4 ms at 60 Hz reference; report quality and any cold work separately. | B,N |
| TEST-PERF-02 | F13 5,000-entity sketch pointer sweep and edit. | p95 pick CPU ≤4 ms, p99 ≤8 ms; pointer-to-submission p95 ≤33.4 ms; no full-sketch hover upload. | B,N |
| TEST-PERF-03 | F17 colored body, face color changes, large selection. | Indexed/selective-split memory and uploads fit budgets; no full deindex unless genuinely required ownership demands it. | B,N |
| TEST-PERF-04 | Separate 5M-triangle, 1,000-body, and 20,000-sketch-entity stress runs. | Bounded jobs/memory, usable cancellation, explicit limited status; do not apply a false 60 Hz promise. | B,N |
| TEST-PERF-05 | Worst warm hover/section/preview sequences and settled idle interval. | No repeated >50 ms warmed frontend tasks; no recurring idle work; allocation profile shows no positive leak slope. | B,N |
| TEST-GRID-01 | Top/oblique grid across projection/FOV/zoom states. | Spacing uses current projected metric; one grid owner; values and origin remain correct. | U,G,N |
| TEST-GRID-02 | Repeated zoom near a 1/2/5 threshold. | Hysteresis prevents flicker; snap spacing changes according to declared policy, never by hidden render-state drift. | U,G |
| TEST-GRID-03 | Hide grid while snap-to-grid stays enabled, enter/exit sketch. | Display and snapping independent; active sketch grid follows preference without overwriting stored model preference. | U,N |
| TEST-GRID-04 | Near-edge-on view, extreme pan, scene rebase, both themes. | Finite mesh/geometry counts; no misleading stale metric; major/minor distinction and label readability preserved. | G,N |
| TEST-CAPTURE-01 | Save thumbnail while idle, after resize, section change, and theme change. | Capture forces/awaits appropriate current successful submission; nonblank correct pixels; bounded output. | G,N |
| TEST-CAPTURE-02 | Capture during unavailable context/disposed engine/oversized output or failed render. | Returns explicit absent/error decoration result; save of user model still succeeds as defined. | U,N |
| TEST-CAPTURE-03 | Compare preserveDrawingBuffer true and false in diagnostic native build. | Production default changed only with native compositor/idle/capture evidence; no claim that all on-demand canvases require preservation. | G,N,P |

## 12. Platform and visual matrix

For small correctness fixtures run the complete Cartesian product of light/dark, perspective/orthographic, supported display mode, section off/on, and relevant DPR. For expensive native geometry/performance fixtures use the explicit high-risk combinations plus pairwise coverage; list combinations exercised rather than claiming exhaustive combinatorial coverage.

At least one native smoke per existing shipped platform is required before asserting that platform's release readiness. A macOS-only result is allowed as a macOS-only milestone. No simulated browser device setting proves physical touchscreen support.

Visual comparisons use stable cameras, lights, assets, viewport size, fonts already provided by the application, and renderer version. Keep per-platform tolerance policies. Do not globally widen a pixel threshold to conceal a local geometric defect. Exclude dynamic text regions only with a written reason and independent semantic assertion. A reviewer approves baseline updates with before/after evidence and the intended design change.

## 13. Evidence artifact layout

The implementation creates `docs/qa/viewport-hardening/` using existing project conventions:

```text
baseline/                 # original red evidence, never overwritten
runs/<run-id>/
  manifest.json           # commit, dirty diff hash, kernel/dependency identity, lanes
  tests.json              # case IDs, result, command, duration, reason for skip
  performance.json        # raw stage samples, summaries, workload/quality metadata
  resources.json          # ownership and allocation counters
  captures/               # actual images and frame/submission correlation
  logs/                   # stdout/stderr, native diagnostics, timeout evidence
fixtures/                 # definitions/seeds and expected semantic facts
qualification.md          # accepted scope, exclusions, failures, evidence links
```

Do not include secrets, absolute private home paths, or user documents in shared evidence. Native machine details should be sufficient for reproduction without exposing unrelated account/device identifiers.

For every failure record: first failing commit or baseline, minimal reproducer, expected/actual observation, relevant identity/projection/quality values, raw output, fix commit/diff hash, and rerun result. Timeout evidence must distinguish missing render, worker delay, invalid currentness, and test harness failure.

## 14. Release decision

A package can be implemented without being accepted. Final release qualification requires all mandatory IDs implemented, all required executable lanes green, no suppressed baseline failure, no untriaged leak or wrong-topology defect, and an explicit physical/native coverage report. Generated source or passing static TypeScript compilation alone does not satisfy any native gate.

Physical-device unavailability is reported as blocked; it does not justify inventing a result or redesigning the test into a mock. Continue useful implementation, but keep the corresponding certification claim absent. The final handoff separates completed code, focused evidence, integrated evidence, native acceptance, and remaining blocked qualification.
