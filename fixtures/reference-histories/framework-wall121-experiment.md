# Framework straight wall diagnostic

This is a partial native-history experiment for the pinned source SHA-256
`6128186260b22ace271e83f096e2c16707b60866b3aa1b3a3f2ea0e38c3c7c09`.
STEPCAF product `FW_EXP_1USBC_FRAME_CLIP_BC_229_` identifies one enclosure
solid. Source and current native exports each contain one BRepCheck-valid solid.
This uses the same OCCT kernel family as the worker, so it is not an independent
geometry oracle or a claim of source equivalence.

In the source-to-native coordinate frame, the straight portion of trimmed wall
face 121 lies on x = 14.19 mm, y = 2.203927026704–26.85921350366 mm,
z = 3.942260869565–4.257549698780 mm. Its adjacent lower plane 88 is
`0.7248498509308111*(x-14.595) + 0.6889068831167106*(z-3.516130434783) = 0`.
Upper plane 90 is `-0.05233595624321788*(x-14.24214960651) +
0.9986295347545595*(z-4.260282743848) = 0`. The diagnostic cut sketches the
plane-derived quadrilateral through x = 15.1 mm, extrudes it along y from
2.203927026704 to 26.8 mm, and mirrors it. Its end is deliberately before the
source's B-spline transition; it does not reconstruct that transition.
`ONECAD_FRAMEWORK_EXPERIMENT_WALL121=1` enables this optional test path after
the rounded inboard experiment. The experiment also requires
`ONECAD_FRAMEWORK_EXPERIMENT_CLIP_PLANES=1`,
`ONECAD_FRAMEWORK_ROUND_CLIP_TOOLS=1`,
`ONECAD_FRAMEWORK_ROUND_CLIP_CORNERS=1`,
`ONECAD_FRAMEWORK_EXPERIMENT_REAR_CLIP_PLANES=1`, and
`ONECAD_FRAMEWORK_EXPERIMENT_INBOARD=1`. It does not change the default model.

On the prior worker export, two same-kernel OCP cuts reported valid results and
reduced native volume by approximately 0.609066 mm³. Two 20³ source/native point
classification grids found 218 to 0 and 1,286 to 37 native-only samples after
the cut, with zero samples newly removed from source. These finite samples do
not prove containment. The same-kernel source/native Boolean partition did not
conserve mass, and its result after the cut was grossly inconsistent; its output
cannot establish agreement. `inspect_framework_residual.py` independently
selects the product by STEPCAF name, checks solid counts and BRep validity,
and reports adaptive integration estimates and conservation discrepancies. The production worker (SHA-256
`4c1b69257302b1c00546fc6f25255727b23125061f6c8ad90cf87c1fdfea7632`)
published both cuts, removing 0.3045428766945406 and 0.30452921077085193 mm³.
The optional test passed undo/redo and cold-worker reopen with an exact mesh
signature and identical volume bits (0 ULP). On the reopened worker export,
the same two finite grids reproduced those counts. Named-source Boolean
comparison reported native-minus-source 2.8484655775 mm³ and
source-minus-native 0.0065968608 mm³; conservation errors were
-0.0015613595 and +0.0003088286 mm³, exceeding the adaptive integration
estimates. Those Boolean values are diagnostic only and do not prove a source
match. The test took 620.95 s under concurrent load; this is not a performance
measurement.
