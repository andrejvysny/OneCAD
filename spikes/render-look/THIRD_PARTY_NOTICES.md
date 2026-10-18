# Third-party notices (render-look spike)

The files listed in the repository's `ATTRIBUTION.md` adapt portions of the projects below. Their
copyright and permission notices follow, as the MIT License requires for substantial portions.

## SolidCraft (github.com/bherbruck/solidcraft @ 6bde126a), used under its MIT option

Adapted in `src/shaders/lines.wgsl` (`expand`), `src/shaders/body.wgsl` (`fs_body` fragment
silhouettes) and `src/render.rs` (`FACE_BIAS`, depth conventions).

```
MIT License

Copyright (c) 2026 SolidCraft contributors

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

## XeGTAO (github.com/GameTechDev/XeGTAO)

Structure and falloff constants followed in `src/shaders/post.wgsl` (`fs_gtao`).

```
MIT License

Copyright (C) 2016-2021, Intel Corporation

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

## Khronos PBR Neutral (github.com/KhronosGroup/ToneMapping), Apache-2.0

`src/shaders/common.wgsl` (`pbr_neutral`) reimplements the published reference operator. The
Apache License 2.0 text is at https://www.apache.org/licenses/LICENSE-2.0; if this code is
carried into a shipped product, include that license and any NOTICE file of the source.
