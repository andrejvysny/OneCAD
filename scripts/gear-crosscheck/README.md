# Gear cross-check harness

Equivalence evidence for OneCAD's gear samplers (`worker/src/kernel/gear/`)
against the `freecad.gears` reference implementation they were ported from.

This is leg 2 of the Gear Generator plan's three-legged oracle strategy:

1. **Closed-form analytic oracles** — pure arithmetic (pitch/addendum/root
   diameters, transverse pitch, lead angle, arc-table radii), asserted in the
   C++ unit tests. Independent of *both* implementations.
2. **This harness** — a parameter sweep diffed against a shipping
   implementation. Catches transcription errors that a self-consistent
   re-derivation would not.
3. **Kernel oracles** — B-rep validity, solid count, volume bounds, curve
   deviation. Arrives with the G1+ worker path.

## This never runs in CI, by design

The reference is **GPL-3.0**. OneCAD is AGPL-3.0, and GPLv3 §13 makes the
one-way derivation lawful — but the reference source stays out of this repo,
out of the build, and out of every test lane. Two reasons:

- **Licensing hygiene.** Nothing in OneCAD's shipped or tested artifacts
  should depend on a GPL checkout being present.
- **Determinism.** The reference pulls in numpy and scipy, and two of its
  quantities come from *unconverged numeric minimisers* (see Tolerances).
  A CI gate on that is a flake generator.

So: run it locally when you touch a sampler, record the result in the work
package, and keep it out of `ci.yml`.

## Running it

Needs `numpy`, `scipy`, and a `freecad.gears` checkout — but **not** FreeCAD
itself. Only the reference's arithmetic is used; none of its `Part` shape
building is touched.

```bash
python3 -m venv /tmp/gearenv
/tmp/gearenv/bin/pip install numpy scipy
git clone https://github.com/looooo/freecad.gears /tmp/freecad.gears

/tmp/gearenv/bin/python3 scripts/gear-crosscheck/crosscheck.py \
    --pygears /tmp/freecad.gears
```

Useful flags: `--show-refusals` (see below), `--verbose` (every mismatch, not
the first 15), `--cxx` (compiler override).

Expected output:

```
cases=2697 compared=2673 refused-by-ours=24 mismatched=0
AGREEMENT: every compared value matches the reference within tolerance.
```

## No build system

`gear_dump.cpp` compiles with a bare `g++` against the sampler sources — no
CMake, no OCCT, no JSON library. That is not a convenience hack; it is a
**standing check on an architectural invariant**. Every file under
`worker/src/kernel/gear/` is required to be pure arithmetic (see
`GearTypes.h`). If this harness ever fails to compile, a sampler has grown a
kernel dependency it is not allowed to have, and the harness says so
explicitly rather than falling back to the CMake build.

## Tolerances

Per-recipe, in `crosscheck.py`:

| recipe    | tol     | why |
|-----------|---------|-----|
| involute  | `1e-9`  | both sides closed form |
| timing    | `1e-9`  | both sides closed form |
| worm      | `1e-9`  | both sides closed form |
| lantern   | `1e-6`  | upstream root-finds φ_min with `scipy.optimize.root` |
| timingT   | `5e-5`  | upstream locates flank endpoints with `scipy.optimize.minimize` |

The two loose rows bound **upstream's** error, not this port's. Both
quantities are solved exactly here — a line/circle intersection and a
bracketed root — and the C++ unit tests hold them to `1e-9` against closed
form. `minimize` on a squared distance is flat at its optimum, so it stops
early; up to `5.1e-6` mm of endpoint error was measured. Tightening these
rows would be asserting upstream's noise.

## Refusals are not failures

`refused-by-ours=N` counts cases where OneCAD returns a graded refusal and the
reference produces *something*. That is the fail-closed rule working: the
reference has no domain guards and will emit inverted or degenerate profiles
rather than stop.

Review them with `--show-refusals` whenever the count changes. At the time of
writing all 24 are the same condition — `module=2, diameter=5, clearance=0.25`
drives a worm's root radius to exactly `0`.

A refusal the reference handles *correctly* would be over-strictness and a
real bug. The count changing without a deliberate guard change is the signal
to look.

## What it found

Recorded because it is the harness's justification, not a changelog:

- **The reference's undercut trim branch is unreachable.**
  `InvoluteTooth.points` computes `s = trimfunc(...)` and guards on
  `isinstance(s, ndarray)`, but `trimfunc` returns a Python *list*, so the
  test never passes and the result is always discarded — every undercut gear
  it has ever produced takes the `nearestpts` fallback. An earlier revision of
  this port implemented the *intended* behaviour and disagreed on **224 of
  2673** cases. `InvoluteMath.h` documents the decision to match reachable
  behaviour instead. Reading the source did not catch this; the sweep did.
