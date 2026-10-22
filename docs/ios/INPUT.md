# iPadOS input routing

Contract for IP05 (ONECAD-452), IP06 (ONECAD-453) and IP19 (ONECAD-466). The
pure state machine lives in `onecad-interaction::touch`; `onecad-ios` adapts
its output to egui events and app commands. UIKit only copies samples
(`onecad_ios.h`). Nothing here claims physical Pencil behavior.

## Pipeline

```
UITouch / UIPencilInteraction / gesture recognizers   (platforms/ios, main thread)
  -> onecad_ios_touch_sample batches (copied)           (C ABI)
  -> touch::Normalizer: chronology, dedupe, predictions, corrections, bounds
  -> touch::Router: one owner per contact, gesture arbitration
  -> RoutedEvent stream
  -> onecad-ios: egui pointer/key events, camera motions, overlays, commands
  -> onecad-ui App::frame (same editor code as macOS)
```

## Normalizer rules

1. Samples are grouped by contact and processed in timestamp order. Within a
   batch UIKit order is preserved for equal timestamps.
2. A coalesced sample equal (time and position) to the following actual sample
   of the same contact is dropped, so the final sample is delivered once.
3. Predicted samples never leave the normalizer as pointer events. Each actual
   batch replaces the contact's prediction list; ended/cancelled contacts clear
   it. Predictions are exposed only as `Overlay::Prediction`.
4. Estimation updates match a retained sample by estimation index (bounded
   ring, 256 per contact). They update optional properties (force, altitude,
   azimuth, roll) and never move an already delivered position or re-emit a
   pointer event. Unmatched updates are counted and dropped.
5. A non-terminal sample older than the contact's last delivered actual sample
   is stale and dropped. Terminal phases (ended, cancelled) are always applied.
6. Samples for an unknown contact without a `began` phase are dropped. A
   contact that began under an older `view_generation` is cancelled when a
   newer generation arrives (rotation/resize) and its later samples dropped.
7. Bounds: 16 live contacts, 4096 samples per batch, 16384 pending. Overflow
   merges consecutive moved samples of one contact (keeps the newest) and
   never drops began/ended/cancelled.

## Ownership and arbitration

Router input per frame: `RouteContext { pencil_only_drawing, drag_authoring,
over_control: fn(pos) -> bool }` where `over_control` comes from the previous
egui frame (pointer over an egui area other than the CAD canvas) and
`drag_authoring` is true while a sketch tool, handle or parameter drag can use
a dragged pointer.

| Contact | Condition | Owner |
|---|---|---|
| Pencil | any | `Author` (pointer down/move/up at the sample position) |
| Finger | starts over a control | `Control` (pointer, so buttons and fields work) |
| Finger | on canvas while a Pencil `Author` contact is live | `Ignored` (camera held stable; controls still work) |
| Finger | on canvas, ends within slop (8 pt, 0.5 s) | `Tap` (pointer down+up at the begin position; double tap counted by egui) |
| Finger | on canvas, exceeds slop, `drag_authoring && !pencil_only_drawing` | `Author` |
| Finger | on canvas, exceeds slop otherwise | `Orbit` (camera motion from the slop exit point, no jump) |
| Finger | a second finger contact arrives | all canvas fingers become `MultiTouch`; native pan/pinch drive navigation; an `Orbit` ends without a jump; an `Author` finger is cancelled |
| Finger | back to one finger after multi-touch | the remaining finger is `Ignored` until lifted |

- One owner per contact for its whole life. An `Author` contact produces
  exactly one pointer press; Pencil never also produces a finger tap or a
  synthesized mouse click.
- Hover (Pencil or pointer, no contact down) produces pointer moves without a
  button and is suppressed while any `Author` contact is live. Hover exit
  clears hover only; it never cancels accepted work.
- Pencil tip double tap is a second tap by the same Pencil (`tap_count` 2) and
  promotes through egui's double-click path (face to body). Barrel double tap
  and squeeze arrive as `onecad_ios_pencil_event` and are handled by the
  accessory policy below; they never author geometry.

## Cancellation

`RoutedEvent::Cancel { contact, reason }` is produced for UIKit cancellation,
`onecad_ios_input_cancel_all`, generation changes, document switches and text
focus. `onecad-ios` maps it to `App::cancel_contact()` (cancels live canvas
drags and pending clicks without leaving the active tool) followed by
`PointerGone`; no release is delivered at the canvas, so no click or drag
commit can result from a cancelled contact.

## Accessory policy (IP19)

| Event | System preference | Action |
|---|---|---|
| Barrel double tap / squeeze | `ignore` | nothing |
| Barrel double tap | `switchPrevious`, `switchEraser` | toggle to the previous sketch tool (no eraser concept) |
| Barrel double tap / squeeze | `showContextualPalette`, `showColorPalette`, `showInkAttributes` | open the adaptive tool palette at the hover position, or at the last Pencil position |
| Squeeze | `runSystemShortcut` | nothing (system handles it) |

Unavailable capabilities stay unavailable; every accessory action has a visible
control equivalent.

## Appendix: trace fixtures

`fixtures/ios/traces/*.json` are deterministic inputs for the normalizer and
router (`onecad_interaction::touch::trace`). Each file:

```json
{
  "name": "<file stem>",
  "description": "free text",
  "context": {"pencil_only_drawing": false, "drag_authoring": false,
              "controls": [[x0, y0, x1, y1]]},
  "steps": [
    {"batch": {"generation": 0, "samples": [ <sample>, ... ]}},
    {"cancel_all": "system_gesture" | "scene_inactive" | "orientation" | "text_focus" | "document_switch" | <ABI code>},
    {"context": { <context, replaces the current one> }},
    {"navigation": true}
  ]
}
```

A sample is `contact`, `phase`, `t` (seconds), `x`, `y` (logical points), all
required, plus optional `kind` (`finger`, `pencil` (default), `pointer`),
`provenance` (`actual` (default), `coalesced`, `predicted`,
`estimation_update`), `tap_count` (default 1), `estimation_index`, `force`,
`max_force`, `altitude`, `azimuth`, `roll`, `hover_z`, `estimated` and
`expects_update` (lists of property names) and `"location": false` (an
estimation update without a location). Enum fields also accept their ABI
numbers. A property is present only when given, which is how a trace models
missing capabilities.

`navigation` models the host's native pan/pinch recognizer beginning. Controls
are inclusive rectangles; a point inside one is `over_control`. After each step
the pending events are routed with the current context.

`fixtures/ios/traces/expected/<name>.txt` holds the canonical routed-event
lines (`touch::trace::describe`, then the normalizer counters) preceded by
`digest <FNV-1a 64>` of those lines. `crates/onecad-interaction/tests/touch_traces.rs`
compares them; `ONECAD_UPDATE_TOUCH_TRACES=1` rewrites them, and the rewrite is
reviewed like code.
