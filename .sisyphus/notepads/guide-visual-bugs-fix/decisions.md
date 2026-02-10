## Guide Visual Bugs Fix — Decisions

- Vertex/Endpoint always wins over Intersection (preserve existing contract)
- Standard 2mm snap radius for crossing detection (no expansion)
- Crosshair renders whenever ≥2 non-collinear active guides intersect (regardless of snap type)
- Fixed screen-space dash pattern: 8px dash, 6px gap
- infiniteLineIntersection duplicated inline in renderer (not abstracting for 2 call sites)

