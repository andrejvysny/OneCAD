# Attribution for the vendored subset

| File | Author/source | License |
|---|---|---|
| crates/geom/src/**, crates/render/src/**, crates/gpu/src/** | SolidCraft contributors; https://github.com/bherbruck/solidcraft, commit a228a5830eedf88856fd392b7db0ff5977f352d6 | MIT OR Apache-2.0 |
| assets/shaders/viewport.wgsl | Original SolidCraft resource data or drawing code, collected without redesign | MIT OR Apache-2.0 |
| crates/gpu/src/pbr.rs | Original SolidCraft WGSL translation of its opaque surface and fixed studio reference; no external shader code | MIT OR Apache-2.0 |
| crates/render/src/lighting/hdr.rs | SolidCraft adaptation of the Radiance decoder row/component/old-RLE state machines in image 0.25.10, https://github.com/image-rs/image | MIT OR Apache-2.0 |

The HDR adapter adds cooperative steps, checked run lengths, zero-run rejection,
width validation, header/input/radiance bounds, and cancellation. Original image
license texts are retained in `licenses/image-0.25.10/`. No images or artwork from
image are copied. SolidCraft's original NOTICE is retained verbatim; references to
other components in that upstream notice do not imply inclusion in this subset.

These focused entries preserve the relevant statements from upstream ATTRIBUTION.md.
