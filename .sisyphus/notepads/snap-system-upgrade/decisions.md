# Snap System Upgrade — Decisions

## Architectural
- Spatial hash cell size = snap radius (2mm)
- Guide lines: transient hover-only, orange dotted
- Angular snap: absolute reference (sketch X-axis = 0 deg), 15 deg increments
- Renderer stays draw-only; all inference math in SnapManager
- SnapResult extended with guideOrigin, hasGuide, hintText (non-breaking defaults)
- findBestSnap gets optional referencePoint param (backward compatible)

