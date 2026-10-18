# Native UI migration: Phase 0 evidence index

Program: Plane ONECAD-255. Recovered Phase 0 reports were written 2026-10-17
against `d76857c2`; the reconciliation below records the user's subsequent
accepted direction. Scope is macOS desktop only; iPad/iOS is out of scope.

The reports contain historical measurements and reported review verdicts with
findings. No runtime gate was rerun for this index. A reported result proves only
that reported probe, at its recorded revision and conditions; it does not make a
Plane task complete or establish native acceptance. ONECAD-261 remains blocked
by ONECAD-256, ONECAD-257 and ONECAD-259.

## Accepted direction and remaining gates

- Native Rust UI with egui/wgpu, subject to native-input gates under ONECAD-257.
  Slint remains historical comparison evidence; selection is not reopened.
- Sidecar worker for production, normal development and normal real-worker CI.
  Preserve the in-process experiment outside required app composition; it is
  not a prerequisite for migration.
- Preserve current visual language, layout, shortcuts and workflows. Retain
  toolbar, shell, inspector and tree/menu semantics alongside keymap and
  modelingInteraction. DOM-specific representations and probes may change.
- Target core/protocol/app/scene/interaction/render/ui boundaries. App exposes
  public contracts without concrete interaction, egui, wgpu or Tauri imports;
  interaction consumes app contracts. Host owns shared GPU device/queue/surface;
  renderer owns private targets. Checked full MESH1 decoding belongs in protocol
  below scene, without wire changes. Synchronous CPU picking retains the
  eight-logical-pixel edge aperture and two-logical-pixel depth slack at hit depth.
- On 2026-10-18 the user approved quality lighting/grid in both themes and
  delegated line choice; blended analytic lines were chosen for continuity.
  egui text weight/native visual parity and golden qualification remain open;
  no goldens are frozen. The user also approved D1 static PlaneGCS with source/
  relink materials, D4 source or source offer before distribution plus an OCCT
  mirror, and D5 "Andrej Vyšný". Packaging, native qualification, compliance and
  publication remain unverified.
- Data-continuity specification is located; implementation and validation remain
  open under ONECAD-274. A located specification is not implemented migration.

| Document | Plane | Evidence and open work |
|---|---|---|
| [licensing.md](licensing.md) | ONECAD-256 | Recovered inventory and packaging analysis. D1/D4/D5 approved 2026-10-18; implementation and release qualification remain unverified. Reviewer findings include LGPL §6(d)/(c) versus §6(a), BOLTS inventory and PlaneGCS notice/About duties. Existing license files do not prove publication or release compliance. D3 was conditional on Slint, which is not selected. |
| [toolkit-spike-egui.md](toolkit-spike-egui.md) | ONECAD-257 | Historical macOS build and measurements. egui selected subject to remaining native-input gates: Japanese IME, clipboard/native-dialog routing and real trackpad/mouse behavior. 120 Hz pacing was not measured. egui text weight/native visual parity remain open. |
| [toolkit-spike-slint.md](toolkit-spike-slint.md) | ONECAD-257 | Historical comparison. Raw winit drag and SF Mono workarounds, conditional IME result and missing off-screen AX rows remain recorded findings. No toolkit reselection follows from this report. |
| [inprocess-worker-spike.md](inprocess-worker-spike.md) | ONECAD-258 | Historical C ABI v3 experiment reported sidecar parity and ctest 206/206. Preserve crash/wedge/lock/logging findings. These are not current gate results. The reported PlaneGCS stdout defect requires separate current validation. |
| [render-look-spike.md](render-look-spike.md) | ONECAD-259 | Historical candidate quality result: 1.77 ms/frame on NEMA at 2560×1600. Quality lighting/grid in both themes approved 2026-10-18; delegated line choice is blended analytic for continuity. Native qualification and goldens remain unverified; none frozen. |
| [data-continuity.md](data-continuity.md) | ONECAD-260; implementation ONECAD-274 | Located specification, not completed implementation. Tests T1–T16 remain unbuilt in the recovered report; OPEN-1 to OPEN-10 and reviewer findings about cross-process `durable_write` and F6's dead-pid fixture require resolution. |
| [../../../spikes/fixtures/README.md](../../../spikes/fixtures/README.md) | — | Historical real-stack MESH1 fixture provenance: NEMA with 67 bodies and a fillet block. Fixture publication/staging remains separate from native acceptance. |
| [ADR-0015](../../adr/0015-native-rust-ui.md) | ONECAD-261 | Accepted native Rust/egui direction; input, packaging and native visual qualification gates remain open. Records target boundaries and behavioral parity. |
| [ADR-0016](../../adr/0016-kernel-hosting-macos.md) | ONECAD-261 | Accepted sidecar direction for production, normal development and real-worker CI. In-process experiment preserved outside required composition. D1/D4/D5 approved 2026-10-18; packaging implementation and qualification remain unverified. |
| [ADR-0017](../../adr/0017-external-addons-deferred.md) | ONECAD-261 | Accepted deferral of external addons; internal module, slot, ownership and ordering laws carry forward. Implementation and verification remain open. |

Adapted third-party code is listed in [`/ATTRIBUTION.md`](../../../ATTRIBUTION.md).
This index does not assert that those files or changes have been published.

## Historical housekeeping findings

The recovered report recorded large spike build trees (Slint 9.8 GB, egui
4.5 GB, mesh-fixture 3.9 GB, render-look 0.5 GB) and no ignore rule at that time.
Those sizes and ignore findings are historical, not a current filesystem audit;
check current `.gitignore` and staging before a separately authorized commit.

The report also recorded interrupted iOS spike apps on simulator devices
`BA049E23-…` and `B1FB875F-…`. Their current presence is unverified. Cleanup is
outside this reconciliation and needs separate authorization.
