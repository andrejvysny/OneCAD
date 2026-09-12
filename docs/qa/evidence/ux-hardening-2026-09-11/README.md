# Native UX hardening evidence — 2026-09-11

This directory preserves the native-stall evidence cited by Addendum E of
`OneCAD-UX-Review-2026-09-11.md`. The source files were copied byte-for-byte
from `/private/tmp` before further rebuilds or relaunches could truncate them.
They are observations, not a root-cause finding.

## Baseline captured

- Git `HEAD`: `80d4e74c8d32b4d847261d7d502dd397286f0788`.
- Worktree: intentionally dirty; 124 porcelain records at capture. Do not use
  this directory as evidence that any pre-existing implementation change is
  owned by the UX-hardening program.
- Sorted `git status --porcelain=v1` SHA-256:
  `0a464aa5ea365eab968a49ca491ae9cca62bffb1e7d11ef6b6121f2c5a9ed9fc`.
- Sorted changed-path SHA-256:
  `e2ebd88f9535e63c8c8744dc41f962def4e35c5965ee3afad11c48997e287a90`.

## Retained files

| Incident | Runtime log SHA-256 | Process sample SHA-256 |
|---|---|---|
| Fillet F | `bb2a088e8cd8bc61e3895ccf0bb487b017c56240186b95cbf5fb33ace1b8a26e` | `5736641869d8130c4cb041bb89795ea1e244a92eb21273df985a69c26dc687a9` |
| Hole H | `618ecdc813795ebd55cbc90d67cdd1a5f75d0f03c1c3621a953d1693f2f255a4` | `c71533948780b938f9353cf80441350e9028f3bd57511aca7e11bd88883166ea` |
| Offset O | `a6e9f442c66da265f6c39ccbb0ef2747d44fef8a9d731a15d299dd7588c1806d` | `3ceacaa1fa164a906b8b6abcec990a4e08c115d9a9bf931d5a28562174c7cc4b` |

The matching file names retain their incident identifier. Recompute hashes
with `shasum -a 256 docs/qa/evidence/ux-hardening-2026-09-11/*` after copying
or moving this evidence.
