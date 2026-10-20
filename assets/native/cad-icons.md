# Native CAD icon geometry

`cad-icons.json` is the authoritative ordered geometry for the native CAD icons.
`cargo xtask bake-cad-icons` projects and fits this geometry, applies the light and
dark palettes, builds arrowheads and markers, and emits the widget SVG registries.
`cargo xtask bake-cad-icons --check` verifies both registries byte for byte.

The source preserves the original design renderer's 150 glyph IDs, including
compact masters selected at sizes up to 20 pixels. Four sidebar glyphs are drawn
in Rust. Every glyph is baked at 14, 16, 18, 20, 24 and 32 pixels in both themes.
The `#fe01fe` gap sentinel and `currentColor` are resolved by the widget host.

The former JavaScript glyph builders were expanded once into unprojected ordered
geometry. This preserves their exact double-precision coordinates, face holes,
shades, line semantics, detail flags, marker and arrow options, and fit bounds.
The source SHA-256 identifies that migration input. No JavaScript runtime or
pre-rendered SVG is used as generator input. Curves remain the same sampled
polylines as in the original renderer; they were not retessellated.

Each glyph has `fit: [left, top, right, bottom]` and an ordered `items` list.
Items use `t` (`f` face, `l` line, `m` marker, `tube`), `d3` (project 3D points),
`pts`, optional `holes`, `fam` (palette family), `sh` (top/left/right shade),
`k` (line or marker semantic), `w` (tube width), and `o` (draw options).
Painter order is significant. Two-dimensional items use the original 24-unit
canvas; three-dimensional items use the original dimetric projection and fit.
