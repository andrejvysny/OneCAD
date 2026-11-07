# Source-grounded solver inputs

These are captured solver **inputs**, not geometry or rendered-output goldens.
The Robonine parts remain imported analytic references. Their licensed exact XBF
inputs and provenance are retained under
`fixtures/reference-derivatives/robonine/analytic` at repository root.

`robonine-grounded-tree.json` captures production-worker-certified clamp and gear
frames, including their small nonzero floating-point axis/origin components. Its
SHA-256 is `2326d18c24a5e7032ff2979c52bedace4a8fb399b4908f1466895146978baafb`.
One grounded body and five tree edges connect six bodies: two rigid rack/clamp
pairs, two free sliders, and one free revolute joint. The independent graph count
is 30 movable coordinates minus 27 independent constraints, giving three DOFs.
The original numerical estimator refused a nonzero column of approximately
4.707e-15; the input must not be rounded to obtain a rank result.

`robonine-endpoint-tree.json` captures the subsequent request to drive both sliders
to -42 mm and the gear to pi radians. Its SHA-256 is
`ce297b3a54679d2c23147a36942e79c30597350f216e15dc53543098fc695725`.
It originally stalled at residual 3.141592553680595 after five iterations because
the angular finite difference crossed the wrapped-error branch. All three free
DOFs are driven, so the independent expected remaining count is zero.

Both inputs retain the original tolerance 1e-9 and maximum 50 iterations. Slider
ranges are explicitly authored kinematic test bounds derived from the measured
84 mm jaw-plane separation. Gear bounds are the principal-angle interval. They
do not certify physical travel, collision safety, or rack-and-pinion coupling.
