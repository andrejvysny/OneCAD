# Upstream sketch completion regression fixture

`qa_plate_hole.onecad` is the unmodified saved baseline from native QA at
revision `ab974bb40b19b3bc109da71f44881d4c240ff862` on 22 October 2026. It contains
a fully constrained 60 × 40 mm XY rectangle, a 4 mm plate extrusion, a 6 mm
through Hole, a face sketch and an 8 mm additive boss extrusion. It is the clean
saved document, not the failed 70 mm edit state.

SHA-256: `a83d1bbac05941030fc97703eea69e9775b2c29326d8c6a91782383c1183d9ec`.

Changing the original width from 60 to 70 mm intentionally exercises a
fail-closed downstream Hole reference. The regression must distinguish accepted
sketch authoring and closed sketch UI from healthy downstream geometry. It must
retain the repair evidence and prove that Undo restores the healthy baseline;
do not lower reference resolution thresholds to make this fixture pass.
