# Licensing: project license, LGPL obligations and toolkit licenses (macOS desktop)

Status: Phase 0 input (Plane ONECAD-256, program ONECAD-255). Written 2026-10-17.
Decision update, 2026-10-18: the user approved D1, D4 and D5 below. Packaging
implementation, relink validation, release compliance and publication remain
unverified. Historical inventory and analysis below retain their 2026-10-17
measurement scope; approval is not evidence that release materials were shipped.

Scope: the macOS desktop app only. The owner dropped the iPad target for now, so the iOS / App
Store analysis is parked (section 3) and nothing in this document depends on it.

## 0. Historical 2026-10-17 changes and inventory

- OneCAD is licensed **MIT OR Apache-2.0** (copyright 2026 Andrej Vyšný).
  `LICENSE-MIT`, `LICENSE-APACHE` (the official apache.org text, unmodified) are at the repo root.
  `package.json` carries `"license": "MIT OR Apache-2.0"`, as does the `[package]` section of every
  Cargo.toml under `src-tauri/` (8 packages).
- `THIRD_PARTY_NOTICES` now lists OCCT, PlaneGCS, Eigen, Boost, nlohmann/json and the frontend
  stack (three.js, React, Tauri, zustand, tailwindcss). The BOLTS entry is kept as it was.
- The full transitive Cargo and npm trees are **not** listed yet. A generated report (for example
  `cargo about`, or a `cargo xtask licences` like SolidCraft's) is a follow-up for the native app.

## 1. What OneCAD links, as built today

These facts were reported as checked on the spike machine on 2026-10-17. They
were not re-audited for the 2026-10-18 decision update.

| Component | License (source checked) | How it gets into the product |
|---|---|---|
| OCCT 8.0.1, tag `V8_0_1`, commit `b8f597c6…` | LGPL-2.1 + Open CASCADE exception 1.0 (`~/.onecad-occt/8.0.1/share/doc/opencascade/LICENSE_LGPL_21.txt`, `OCCT_LGPL_EXCEPTION.txt`) | Shared libraries. `otool -L worker/build/onecad-worker` lists 49 `@rpath/libTK*.8.0.dylib` lines. `scripts/bundle-dylibs.sh` copies them into `Contents/Frameworks/`. |
| PlaneGCS (FreeCAD `3f49f3f0…`, modified) | LGPL-2.1-or-later (SPDX headers, `worker/third_party/planegcs/LICENSE`) | **Static.** `add_library(planegcs STATIC …)` is linked into `worker_core`, and `worker_core` into `onecad-worker`. `nm -C` shows `GCS::System::*` as defined symbols inside the worker binary. |
| Eigen 5.0.1 | MPL-2.0, with some BSD, Apache and MINPACK files (`/opt/homebrew/opt/eigen/COPYING.README`). No LGPL file is in the 5.0.1 set. | Header-only. Compiled into the worker. |
| Boost 1.90 | BSL-1.0 (header notices) | Header-only (Graph). Compiled into the worker. |
| nlohmann/json 3.12.0 | MIT (SPDX header in `json.hpp`) | Vendored header. |
| three.js 0.185.1, React 19.2.7, zustand, tailwindcss | MIT (`node_modules/*/package.json` and LICENSE) | Webview bundle. This goes away with the native UI. |
| Tauri 2.11.5, `@tauri-apps/api` 2.11.1 | Apache-2.0 OR MIT (crate metadata, LICENSE files) | App crate. This goes away with the native UI. |

The release app also embeds a SHA-256 manifest of the sidecar and refuses a worker whose bytes
differ (`src-tauri/src/worker/manifest.rs`, `verify_binary`). That manifest pins the worker binary
only. It does not pin the OCCT dylibs.

## 2. Desktop (macOS) obligations today

**OCCT (shared library, LGPL-2.1 §6(b)).** Dynamic linking through a shared-library mechanism is
the simple case. What we must ship or do:

1. Include the LGPL-2.1 text and the OCCT exception text in the app bundle, for example under
   `Contents/Resources/licenses/`, along with `THIRD_PARTY_NOTICES`.
2. Give the "prominent notice" the OCCT exception requires: say that OneCAD makes use of Open CASCADE
   Technology. Put this in the About screen and the notices file. The notices file already has it.
   The About screen must add it.
3. Make the OCCT source available (§4/§6). It is unmodified upstream source, and the exact tag,
   commit and build options are recorded. The safest practice is to attach, or mirror, the exact
   `V8_0_1` tarball next to each release, because a link to a third party's server is weaker than
   "accompany or offer".
4. Users must be able to swap in a modified OCCT build. Today they can: replace the dylibs in
   `Contents/Frameworks/` and re-sign ad hoc. The worker's SHA pin does not cover those dylibs.
   One watch item: a notarized build with hardened runtime and library validation would refuse
   dylibs signed by someone else. LGPL-2.1, unlike LGPLv3, has no "Installation Information"
   clause, so re-signing the bundle yourself remains a valid path. Still, document it.

**PlaneGCS (static; approved D1 direction, 2026-10-18).** Keep static linking
with corresponding source and relink materials. The recovered analysis below
was corrected by review: source offered from the binary distribution location
needs the §6(d) route (or a qualifying §6(c) offer), rather than a bare source
link being treated as §6(a) compliance. The complete shipped source/build/relink
materials and their availability must be validated before distribution.

**Historical §6(a) analysis, with the distribution-route correction above.** For a statically linked LGPL library, §6(a) requires the
distributor to ship, or offer, the Library's complete source including our changes. It also
requires "the complete machine-readable 'work that uses the Library', as object code and/or source
code", so the user can modify the Library and relink. §6 also requires our terms to allow
modification and reverse engineering. MIT and Apache-2.0 both allow that.

- **Corresponding-source material identified by the historical analysis.** OneCAD's own
  source, the modified PlaneGCS, and the CMake and scripts that build the exact shipped worker and
  app are all in the repository. The review requires a qualifying distribution route as stated above; merely
  identifying these files does not demonstrate release compliance. The proposed
  corresponding-source approach assumes (a) every shipped binary is built from a tagged public commit, (b) that tag is linked from
  the release, and (c) the build actually reproduces from it. A separate object-file relink kit is
  then not needed.
- **Caveat 1, timing.** The repository is not public yet. Until it is, any binary given to anyone
  else, including beta testers, must come with the source or a written offer. The
  simplest rule: do not distribute binaries before the repository is public.
- **Caveat 2, the SHA pin.** A user who relinks the worker must also rebuild the app, because the
  release app embeds the worker's SHA. With full source this is still "modify and relink". It is a
  lockstep check, not an added restriction. Document the two-pass build (`scripts/package-macos.sh`)
  as the relink procedure.
- **Option: make PlaneGCS a shared library.** This is a one-line CMake change
  (`STATIC` → `SHARED`) plus bundling one more dylib. It moves PlaneGCS to the §6(b) path like
  OCCT, so compliance no longer depends on OneCAD's own source staying public. It is only worth
  doing if a closed-source build is ever possible. D1 selects **keep it static**, with corresponding source/relink materials.
  Publication and reproducibility remain unverified.

**MPL-2.0 (Eigen).** We ship Eigen-derived code only as compiled object code. MPL §3.2 requires us
to tell recipients where the Eigen source is. The notices file does that. Optional hardening:
define `EIGEN_MPL2_ONLY` in `worker/cmake`. This is not my path, so it is recommended but not done.
**BSL-1.0 (Boost)** requires no notice for object code. We list it anyway.

## 3. iOS / iPadOS: parked

The iPad target is out of scope for now (owner decision). Research done before that decision, kept
only as a starting point if iPad returns: on iOS the worker would be in-process, so OCCT and
PlaneGCS ship inside the app; the real risk is App Store terms against LGPL-2.1 §10, not the
linking mode; LGPL-2.1 dynamic frameworks (MobileVLCKit) and OCCT itself (Open Cascade's CAD
Assistant) are on the store; the leaning was static linking plus public tagged source as relink
material and a custom EULA clause. This is **not** a recommendation and needs fresh research and
legal review if iPad is revived. Sources are kept at the end under "Parked iOS sources".

## 4. Slint license check against an MIT/Apache app

Slint is offered as `GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0`
(slint-ui/slint `LICENSE.md`, read 2026-10-17).

- **GPLv3: not acceptable.** Our MIT and Apache-2.0 source is GPLv3-compatible, so the source could
  stay MIT/Apache, but every distributed binary would be GPLv3. That defeats the permissive license
  for every downstream binary and would bar any later closed or store-distributed build.
- **Royalty-free 2.0: usable.** It grants use, reproduction and modification in desktop, mobile
  (explicitly "mobile phone or tablet") and web applications. Embedded systems are excluded. Other
  conditions: we must not distribute Slint alone, must not expose Slint's APIs from our app, and
  must keep license notices. Attribution is required: either the `AboutSlint` widget in an About
  dialog reachable from the top-level menu (or the splash screen), or the Slint badge on a public
  web page such as the download page. OneCAD keeps MIT/Apache. Slint's own terms are not OSI
  open-source, so forks and redistributors inherit the attribution and no-embedded conditions, and a
  fork targeting an embedded device would need GPLv3 or a commercial license. LGPL check: §6 needs
  our terms to allow modification and reverse engineering of the work that uses PlaneGCS. On
  desktop PlaneGCS lives in the separate worker process, not in the Slint UI binary, so Slint's
  terms do not touch it. The royalty-free text (re-read 2026-10-17) does not mention reverse
  engineering either way.
- **Commercial (Software 3.0): paid**, with size-tiered plans (Startup/Individual, Small
  Enterprise, Enterprise). Not needed for this project.

**Historical conditional D3:** Slint would require the Royalty-free 2.0 route
with `AboutSlint`; GPLv3 was ruled out. egui is selected, so this condition is
not active and does not reopen toolkit selection.

## 5. egui / eframe / wgpu / winit

Crate metadata in `~/.cargo/registry`: egui 0.36.2, eframe 0.36.2, egui-wgpu 0.36.2 and wgpu 29.0.3
are `MIT OR Apache-2.0`. **winit 0.30.13 is `Apache-2.0` only.** That is compatible with an
MIT/Apache app, but the binary then contains Apache-2.0 code: ship its license text and any NOTICE
content in the generated license report. None of these crates is copyleft. No sign-off is needed.

## 6. Adapting SolidCraft code

SolidCraft (`/Users/andrejvysny/workspace/solidcraft`) is `MIT OR Apache-2.0`, "Copyright (c) 2026
SolidCraft contributors". It has a NOTICE file. Rules for every file we adapt:

1. Take it under the same dual license and keep it under `MIT OR Apache-2.0`.
2. Keep a header in each adapted file:
   `Adapted from SolidCraft <source path> (MIT OR Apache-2.0), Copyright (c) 2026 SolidCraft contributors; modified for OneCAD.`
   Apache-2.0 §4(b) requires changed files to carry a prominent change notice. MIT requires the
   copyright and permission notice to be kept.
3. Add one row per file to `/ATTRIBUTION.md` (source path | license | our path). Append only.
4. On the first adaptation, add a "SolidCraft" entry to `THIRD_PARTY_NOTICES` and carry the
   relevant SolidCraft NOTICE lines (Apache §4(d)).
5. Check each file's own provenance before copying. SolidCraft contains code from CADCraft and
   GridCraft (MIT/Apache), the `image` crate (MIT/Apache), vendored truck-shapeops (Apache-2.0 only,
   © RICOS) and SIL OFL fonts. Copy none of the truck or vendor code and none of the fonts without
   their own terms. Copy no Autodesk or Fusion vocabulary or assets (SolidCraft's NOTICE disclaims
   them).

No SolidCraft file has been adapted in this step, so no ATTRIBUTION.md row was added.

## 7. Owner decisions (2026-10-18) and remaining qualification

- **D1 approved:** keep PlaneGCS static and provide corresponding source/relink
  materials. Validate the tagged source, build instructions and two-pass relink
  procedure against each shipped binary; implementation remains unverified.
- **D2 iOS packaging:** parked with the iPad target. No decision needed now.
- **D3 inactive:** egui selected; historical Slint conditions remain evidence only.
- **D4 approved:** no binary distribution, beta builds included, before
  corresponding source or a qualifying source offer is available to recipients. Mirror the OCCT `V8_0_1` tarball with each release.
- **D5 approved:** canonical copyright spelling is "Andrej Vyšný". This decision
  does not claim all metadata or published artifacts have been reconciled.

No decision above proves packaging/native qualification or source/binary
publication. Those remain unverified; this record does not close ONECAD-256.

## Sources

- OCCT license and exception: installed files under `~/.onecad-occt/8.0.1/share/doc/opencascade/`.
- LGPL-2.1 text: `worker/third_party/planegcs/LICENSE`; https://www.gnu.org/licenses/lgpl-2.1.html
- Slint licenses: https://github.com/slint-ui/slint/blob/master/LICENSE.md ,
  https://github.com/slint-ui/slint/blob/master/LICENSES/LicenseRef-Slint-Royalty-free-2.0.md ,
  https://slint.dev/pricing (read 2026-10-17)

### Parked iOS sources

- Apple Standard EULA: https://www.apple.com/legal/internet-services/itunes/dev/stdeula/ (© 2026)
- LGPL on iOS discussion: https://news.ycombinator.com/item?id=38965577 ,
  https://news.ycombinator.com/item?id=21648542 , https://loufranco.com/blog/lgpl-and-the-iphone ,
  https://developer.apple.com/forums/thread/702873 ,
  https://github.com/harflabs/SwiftVLC/issues/158 (returned 404 when fetched; cited from the search result),
  https://wiki.qt.io/Licensing-talk-about-mobile-platforms ,
  https://lists.qt-project.org/pipermail/development/2014-July/017532.html
- App Store and GPL "further restrictions": https://lwn.net/Articles/396925 ,
  https://lists.qt-project.org/pipermail/interest/2012-February/000921.html
- VLC precedent: https://images.videolan.org/press/ios2out.html , https://www.cultofmac.com/?p=201703
- OCCT on the App Store: https://apps.apple.com/us/app/cad-assistant/id984561628 ,
  https://dev.opencascade.org/content/open-cascade-cad-assistant-ios-platform
