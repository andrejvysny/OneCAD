# Gemini Research

Advanced Physically Based Rendering and Visualization Architectures for Computer-Aided Design: A Comprehensive Implementation Framework1. Introduction: The Convergence of Engineering Precision and Visual FidelityThe discipline of Computer-Aided Design (CAD) is currently navigating a significant inflection point, transitioning from a historical focus on strictly functional, parametric wireframe and flat-shaded representations to high-fidelity, Physically Based Rendering (PBR). This paradigm shift is not merely an aesthetic evolution; it represents a fundamental change in the engineering design process, where visual verification of material properties, surface finishes, and lighting interactions becomes integral to the product development lifecycle rather than a post-process marketing step.1 The ability to visualize a mechanical assembly with "true-to-life" accuracy allows engineers to assess aesthetic defects, ergonomic implications, and manufacturing feasibility—such as the visibility of injection molding sink marks or the specular behavior of machined aluminum—before a physical prototype is ever constructed.This report provides a rigorous, evidence-backed analysis of the theoretical foundations and software engineering methodologies required to implement a state-of-the-art PBR rendering engine tailored for CAD. Unlike entertainment-focused engines where artistic license allows for approximations, CAD visualization demands a rigorous adherence to radiometric accuracy and geometric precision.2 The analysis synthesizes data from industry leaders such as Blender and Shapr3D, and formulates a specific implementation roadmap using C++, Qt 6, and OpenGL 4.1 (Core Profile), integrated with the Open CASCADE Technology (OCCT) kernel. This specific technology stack is chosen to maximize cross-platform compatibility—particularly with macOS, where OpenGL 4.1 remains the ceiling for native support—while leveraging the robust B-Rep manipulation capabilities of OCCT.4The report is structured to guide a development team from the theoretical radiometry of microfacet surfaces through to the low-level management of the OpenGL render loop and the architectural integration of complex CAD data structures. It addresses critical engineering challenges such as Order-Independent Transparency (OIT) for complex assemblies, Reverse-Z depth buffering for massive scale differences, and the nuanced handling of color management spaces (AgX vs. Khronos PBR Neutral) to ensure that the "red" on the screen matches the "red" of the manufactured plastic.62. Theoretical Foundations of Physically Based Rendering in EngineeringPhysically Based Rendering (PBR) fundamentally differs from legacy ad-hoc lighting models (such as Blinn-Phong or Gouraud) by strictly adhering to the laws of physics, specifically the principles of energy conservation and the microfacet theory of surface interaction.3 In a CAD context, PBR provides a "material truth" that empirical models cannot; a metallic surface in a PBR engine behaves like metal because the math simulates the electromagnetic interaction of light with conductors, not because a specular color was arbitrarily set to bright white.2.1 The Radiometry of Surface InteractionAt the core of the PBR pipeline is the Bidirectional Reflectance Distribution Function (BRDF), denoted as $f_r(l, v)$, which defines the ratio of reflected radiance to incident irradiance. The rendering equation, which must be approximated in real-time for interactive CAD viewports, is expressed as:$$L_o(p, \omega_o) = L_e(p, \omega_o) + \int_{\Omega} f_r(p, \omega_i, \omega_o) L_i(p, \omega_i) (\omega_i \cdot n) d\omega_i$$For real-time implementation, particularly in OpenGL 4.1, the Cook-Torrance specular BRDF is the industry standard.9 This model is favored for its ability to unify the representation of both dielectrics (plastics, ceramics, wood) and conductors (metals) within a single mathematical framework, reducing the shader complexity required to render diverse CAD assemblies.102.1.1 The Cook-Torrance Microfacet ModelThe specular component of the BRDF is calculated as:$$f_{specular} = \frac{D(h) F(v, h) G(l, v, h)}{4 (n \cdot l) (n \cdot v)}$$This equation implies that the surface is composed of countless microscopically small mirrors (microfacets). The roughness of the material is determined by the statistical alignment of these facets. The three key terms—$D$, $F$, and $G$—must be selected carefully to balance performance with the visual fidelity required for engineering evaluation.1. The Normal Distribution Function $D(h)$:The Normal Distribution Function approximates the probability that the microfacets are aligned with the halfway vector $h$, causing a specular reflection. For CAD materials, the Trowbridge-Reitz GGX distribution is universally preferred over the older Beckman or Blinn-Phong distributions.8 The mathematical rationale lies in the "tail" of the distribution curve; GGX has a longer tail, meaning the specular highlight falls off more gradually. This is critical for rendering machined metals and polished polymers, where the "glow" around the highlight communicates the surface finish quality to the engineer. A sharp cut-off (like in Blinn-Phong) makes materials look plastic and artificial, masking potential surface curvature details.2. The Fresnel Equation $F(v, h)$:The Fresnel equation describes the ratio of light reflected versus refracted (absorbed or scattered) as a function of the viewing angle. The Schlick approximation is the standard for real-time performance.11 It relies on $F_0$, the surface reflectance at normal incidence ($0^\circ$).A critical distinction in CAD rendering is the strict separation of material physics:Conductors (Metals): Have a high $F_0$ (0.5 to 1.0) and are often colored (e.g., Gold, Copper). They have no diffuse component because all refracted light is absorbed by the free electrons in the metal.Dielectrics (Non-metals): Have a low, achromatic $F_0$ (typically 0.04 for plastic, glass, paint) and a significant diffuse component.In legacy CAD renderers, users often manually set "specular color." In a PBR workflow, this is physically incorrect. The engine must enforce these rules: if a material is metal, its "diffuse" color is black, and its "albedo" drives the $F_0$. If it is dielectric, the $F_0$ is fixed (usually 0.04), and albedo drives the diffuse color.133. The Geometry Function $G(l, v, h)$:This term accounts for self-shadowing (light blocked before reaching a facet) and self-masking (light blocked after reflection). The Smith-GGX height-correlated visibility function is the most accurate approximation for engineering materials.9 Simpler models often result in edges appearing too dark or too bright at grazing angles. For a CAD engineer verifying the silhouette of a car body or a consumer electronic device, correct edge rendering is vital, making the Smith-GGX model non-negotiable despite its slightly higher computational cost.2.2 The Metallic-Roughness WorkflowThe computer graphics industry has largely standardized on the Metallic-Roughness workflow over the Specular-Glossiness workflow.14 This decision is driven by texture memory efficiency and ease of authoring, which is crucial for CAD users who may not be expert texture artists.ParameterDescriptionData Structure (glTF 2.0 Standard)Physical Meaning in CADBase Color (Albedo)Defines the diffuse color for dielectrics and the specular color ($F_0$) for metals.RGB Texture (sRGB)The paint color or the intrinsic metal color (e.g., Anodized Aluminum).MetallicA binary mask (0 or 1) blending between dielectric and conductor physics.Blue Channel of ORM Texture 16Distinguishes between metal parts and plastic/rubber components.RoughnessControls the spread of the specular highlight (smooth vs. matte).Green Channel of ORM Texture 16Represents the surface finish (e.g., Polished, Sandblasted, Matte).Ambient OcclusionMicro-scale shadowing data.Red Channel of ORM Texture 16Simulates accumulated dirt or micro-cavities in cast parts.Adopting the glTF 2.0 material specification ensures interoperability. A CAD model exported from the application should look identical in a web viewer, an AR application (like iOS Quick Look), or a downstream rendering tool like KeyShot. The Khronos Group's standardization of these parameters allows the CAD tool to act as a reliable "source of truth" for the product's visual appearance.142.3 Color Management: AgX vs. PBR NeutralA critical, often overlooked aspect of CAD rendering is color accuracy. While entertainment rendering prioritizes a "filmic" look with high contrast and saturation roll-off, engineering design requires predictable color reproduction. If a designer specifies a specific RAL or Pantone color for a plastic part, the renderer must display that color accurately, not a "movie-like" version of it.2.3.1 The AgX TransformBlender 4.0 introduced AgX as the default view transform, replacing the older "Filmic" transform.18 AgX improves the handling of high-dynamic-range (HDR) imagery by desaturating bright colors as they approach white, mimicking the response of chemical film. This solves the "Notorious Six" problem where highly saturated colors (e.g., bright LEDs or lasers) would clip unnaturally to primary colors (pure red, pure green, etc.).19However, for product visualization, AgX can be problematic. It intentionally alters hues and saturation to fit within the dynamic range. A "Coca-Cola Red" might render as slightly orange or desaturated under bright lighting to preserve detail. For a marketing render, this is acceptable; for design verification, it is a flaw.62.3.2 Khronos PBR Neutral Tone MapperTo address the precision needs of e-commerce and CAD, the Khronos Group released the PBR Neutral Tone Mapper.7 Unlike AgX, this tone mapper prioritizes the preservation of the base color (sRGB) under standard lighting. It creates a 1:1 mapping for well-exposed colors while compressing highlights smoothly.Recommendation: A robust CAD rendering engine must implement both. Khronos PBR Neutral should be the default for the modeling viewport to ensure the engineer sees the "correct" material color. AgX should be available as a "Cinematic" or "Marketing" toggle for creating promotional visuals where mood and realism supersede strict colorimetric accuracy.203. Comparative Analysis: State of the Art (Blender & Shapr3D)Analyzing existing market leaders provides a benchmark for feature sets, rendering quality, and user experience (UX).3.1 Blender: The Dual-Engine ParadigmBlender utilizes two distinct engines that represent the spectrum of rendering possibilities: Eevee (Real-Time) and Cycles (Path Tracing), plus a third Workbench engine for pure modeling.Eevee (Rasterization): Eevee employs deferred rendering and heavy screen-space effects (SSAO, SSR) to approximate global illumination.22 It uses shadow maps and light probes for indirect lighting.23 While fast, the lack of true ray tracing limits its accuracy for complex refractions (glass) and inter-reflections, which are common in mechanical assemblies (e.g., looking through a transparent housing at internal gears).Cycles (Path Tracing): Cycles uses unidirectional path tracing with multiple importance sampling.24 It offers "ground truth" realism but is computationally expensive, often requiring seconds or minutes to clear noise, which interrupts the design flow.Workbench Engine: Crucially, Blender maintains a "Workbench" engine designed specifically for modeling.26 It features distinct object differentiation (random colors), cavity shading to highlight topology (ridges and valleys), and mattecaps.Insight for CAD Implementation: High-end CAD software should emulate this separation. The "Working View" should mimic Blender's Workbench (flat shading, cavity detection, sharp edges for topology check), while the "Visualization View" should mimic Eevee (PBR, shadows, ambient occlusion) for aesthetic validation. Trying to model in a fully ray-traced view (Cycles-like) is often counterproductive due to visual noise and performance lag.3.2 Shapr3D: The Parasolid-Metal IntegrationShapr3D represents the pinnacle of modern CAD rendering on mobile (iPad) and desktop architectures, specifically in how it handles the transition from parametric kernel to visual mesh.Technology Stack: It utilizes the Parasolid kernel for geometry (same as Siemens NX and SolidWorks) and renders via Metal (macOS/iPad) and DirectX (Windows).27 This allows it to leverage hardware-specific optimizations that a generic OpenGL implementation might miss.Visualization Mode: Shapr3D isolates visualization into a separate workspace.28 This mode simplifies the UI, exposing only material application, environment rotation, and depth-of-field controls.28 It removes the "clutter" of sketches and construction planes.Material System: The library is curated rather than procedural. It offers drag-and-drop presets (metals, plastics, woods) that map automatically to the B-Rep faces.28 It does not expose complex shader graphs to the user.Integration: A significant innovation is the integration of AR export (USDZ) directly from the visualization workflow.29 This allows an industrial designer to place a virtual prototype on a real desk immediately.Comparison Insight: Shapr3D succeeds because it restricts user freedom to ensure consistency. It does not allow users to break the PBR laws (e.g., creating a metal with a diffuse color). A custom CAD implementation should follow this "curated sandbox" approach. Engineers are not texture artists; they need materials that "just work."4. Architectural Specification: C++, Qt 6, and OpenGL 4.1Building a rendering engine that rivals these tools requires a robust software architecture. The proposed stack is C++17/20 for core logic, Qt 6 for the application framework and UI, Open CASCADE (OCCT) for the B-Rep geometry kernel, and OpenGL 4.1 Core Profile for rendering.4.1 Why OpenGL 4.1 Core Profile?In an era of Vulkan, Metal, and DirectX 12, choosing OpenGL 4.1 appears regressive. However, it is a strategic pragmatism.macOS Support: Apple deprecated OpenGL years ago, freezing support at version 4.1. To run natively on macOS without the overhead of MoltenVK (Vulkan-to-Metal wrapper), the application must adhere strictly to the OpenGL 4.1 Core Profile specs.5Simplicity vs. Control: While Vulkan offers lower-level control, the complexity overhead for a CAD application (which is CPU-bound by geometry kernels, not GPU-bound by draw calls like a game) is rarely justified. OpenGL 4.1 provides sufficient features (Uniform Buffer Objects, Floating Point Textures, Instancing) to implement modern PBR.4.2 Qt 6 Integration StrategyQt 6 introduces significant changes to rendering integration. The QOpenGLWidget is the standard approach, but for high-performance CAD, we must bypass Qt's internal rendering loop for the 3D viewport to ensure low latency.4.2.1 The Decoupled Render LoopThe rendering loop must be decoupled from the GUI thread to prevent UI freezes during heavy mesh tessellation or shader compilation.31Main Thread (GUI): Handles Qt Events (mouse clicks, key presses), Signal/Slot connections, and OCCT data manipulation (TDocStd_Document modifications).Render Thread: Owns the OpenGL Context. Executes AIS_InteractiveContext::UpdateCurrentViewer() and swaps buffers. It waits for a signal from the main thread that data is ready, renders the frame, and signals back.To integrate OCCT, we subclass QOpenGLWidget but override the paint event to delegate strictly to the OCCT OpenGl_GraphicDriver.32C++// Pseudocode for specialized QOpenGLWidget initialization
void QCadWidget::initializeGL() {
    // 1. Retrieve handle to the window created by Qt
    // We strictly use the underlying platform window handle
    Handle(Aspect_Window) aWindow = new Aspect_NeutralWindow();
    aWindow->SetNativeHandle((Aspect_Drawable)this->winId());
    
    // 2. Initialize OCCT Graphic Driver with the current context
    // This allows OCCT to use the OpenGL context created by Qt
    Handle(OpenGl_GraphicDriver) aDriver = new OpenGl_GraphicDriver(GetDisplayConnection());
    
    // 3. Create V3d_Viewer and V3d_View
    // The V3d_View manages the camera, lights, and grid
    m_viewer = new V3d_Viewer(aDriver);
    m_view = m_viewer->CreateView();
    m_view->SetWindow(aWindow);
    
    // 4. Configure Low-Level OpenGL State
    // Crucial for 4.1 Core Profile compliance on macOS
    if (!aWindow->IsMapped()) aWindow->Map();
    m_view->SetShadingModel(Graphic3d_TOSM_PBR); // Enable PBR mode in OCCT
}
4.3 OCCT Data Pipeline (B-Rep to Mesh)OCCT stores geometry mathematically (B-Rep: Boundary Representation). To render this, it must be tessellated into triangles. The BRepMesh_IncrementalMesh algorithm is responsible for this.34Crucial Optimization: Dynamic TessellationCAD models contain curves that require varying tessellation density. A static mesh is insufficient. We must control two key parameters:Linear Deflection: The maximum distance (sagitta) between the true curve and the tessellated chord.Angular Deflection: The maximum angle between normals of adjacent triangles.For high-quality PBR rendering, the default tessellation is often too coarse, leading to faceted highlights. The implementation must implement a Dynamic Level of Detail (LOD) system. When the user enters "Visualization Mode," the application should re-tessellate the B-Rep with a finer linear deflection (e.g., < 0.001mm) to ensure smooth specular highlights across curved surfaces.345. Advanced Rendering Features ImplementationThis section details the specific algorithms and shader techniques required to achieve realistic visuals within the constraints of the defined architecture.5.1 Implementing the PBR Shader in GLSL 4.1The PBR shader requires access to pre-filtered environment maps for Image-Based Lighting (IBL). Since ray tracing is expensive, we rely on the Split-Sum Approximation popularized by Epic Games. This splits the lighting integral into two parts that can be pre-calculated.5.1.1 Image-Based Lighting (IBL) InfrastructureWe cannot rely solely on analytical lights (point/spot/directional). Realism comes from the environment reflections.Irradiance Map: A convoluted cubemap storing the average incoming light for the diffuse component.Prefiltered Specular Map: A mip-mapped cubemap storing environment reflections at varying roughness levels (Roughness 0 = Mip 0, Roughness 1 = Max Mip).9BRDF Integration Map (LUT): A 2D Look-Up Table texture that stores the scale and bias for the Fresnel term based on roughness and viewing angle ($N \cdot V$).8GLSL 4.1 Implementation Note: OpenGL 4.1 requires explicit binding of texture units. The shader will sample the prefiltered map using textureLod based on the material's roughness.OpenGL Shading Language// GLSL 4.1 Fragment Shader Snippet for PBR
// Inputs: Texture maps bound to specific units
uniform samplerCube uPrefilterMap;
uniform sampler2D uBrdfLUT;
uniform samplerCube uIrradianceMap;

vec3 fresnelSchlickRoughness(float cosTheta, vec3 F0, float roughness) {
    return F0 + (max(vec3(1.0 - roughness), F0) - F0) * pow(1.0 - cosTheta, 5.0);
}

void main() {
    // Standard PBR inputs derived from G-Buffer or forward pass
    vec3 N = normalize(vNormal);
    vec3 V = normalize(uCamPos - vPos);
    vec3 R = reflect(-V, N); 
    
    // 1. Fresnel Calculation
    vec3 F = fresnelSchlickRoughness(max(dot(N, V), 0.0), F0, roughness);
    
    // 2. Diffuse/Specular Energy Conservation
    vec3 kS = F;
    vec3 kD = 1.0 - kS;
    kD *= 1.0 - metallic; // Metals have no diffuse
    
    // 3. Diffuse IBL
    vec3 irradiance = texture(uIrradianceMap, N).rgb;
    vec3 diffuse = irradiance * albedo;
    
    // 4. Specular IBL (Split-Sum Approximation)
    const float MAX_REFLECTION_LOD = 4.0;
    vec3 prefilteredColor = textureLod(uPrefilterMap, R, roughness * MAX_REFLECTION_LOD).rgb;
    vec2 brdf  = texture(uBrdfLUT, vec2(max(dot(N, V), 0.0), roughness)).rg;
    vec3 specular = prefilteredColor * (F * brdf.x + brdf.y);
    
    // 5. Combination
    vec3 ambient = (kD * diffuse + specular) * ao;
    
    fragColor = vec4(ambient + Lo, 1.0); // Lo is direct lighting from analytical lights
}
5.2 Handling Transparency in CAD: Order-Independent TransparencyTransparency is notoriously difficult in rasterization. CAD models often feature complex assemblies with nested transparent housings (e.g., a plastic casing containing a PCB, containing a glass display, containing an LED).5.2.1 The Artifact ProblemStandard alpha blending (glBlendFunc(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA)) requires strict back-to-front sorting of triangles. For intersecting geometry—which is incredibly common in CAD assemblies (e.g., interference fits)—sorting is mathematically impossible at the triangle level without splitting geometry. This leads to artifacts where the "inside" of a part renders on top of the "outside".365.2.2 Solution: Weighted Blended Order-Independent Transparency (OIT)For real-time performance in a CAD viewport, Weighted Blended OIT is the recommended architectural choice.37 It is an approximation that calculates a weighted average of transparent surfaces based on their depth and opacity, rather than their strict sort order.Algorithm Steps:Pass 1 (Accumulation): Render transparent geometry to two floating-point textures (e.g., GL_RGBA16F).Texture 1 accumulates: vec4(color.rgb * alpha, alpha) * weight.Texture 2 accumulates: weight (usually just the alpha coverage).The weight function is key; it usually falls off with depth ($z$) so that closer surfaces contribute more to the color.39Pass 2 (Composition): Render a full-screen quad. Read the accumulated textures. Divide the accumulated color by the accumulated weight to normalize. Composite this result onto the opaque background buffer.While Depth Peeling 40 offers pixel-perfect accuracy (peeling away layers one by one), it requires $N$ geometry passes for $N$ layers of transparency. In a CAD assembly with 20 overlapping transparent layers, this causes a massive frame rate drop. Weighted Blended OIT operates in a single geometry pass, making it the only viable option for manipulating complex assemblies smoothly.385.3 High-Fidelity Edge Rendering (Wireframe-on-Shaded)In CAD, viewing the topological edges (wireframe) overlaid on the shaded model is essential for the engineer to understand the form and topology. Standard OpenGL lines (GL_LINES) suffer from z-fighting (stitching artifacts where the line dives into the surface) and aliasing.5.3.1 The Legacy Solution: Polygon OffsetThe traditional method is glPolygonOffset, which pushes the shaded geometry slightly back in the depth buffer to let the wireframe render on top.41 This is fragile; it depends on the depth buffer precision and often causes artifacts at grazing angles where the depth slope is high.5.3.2 The Modern Solution: Screen-Space Barycentric WireframesA superior approach involves passing barycentric coordinates to the fragment shader. This creates high-quality, anti-aliased lines of constant width, regardless of zoom level, without z-fighting.43Vertex Attribute Setup: For every triangle, assign barycentric coordinates to the vertices: $v_0=(1,0,0)$, $v_1=(0,1,0)$, $v_2=(0,0,1)$. This requires duplicating vertices (un-sharing them) so that each triangle has its own unique barycentric domain.Fragment Shader Logic: The rasterizer interpolates these coordinates. At any pixel, the distance to the nearest edge is the minimum of the three barycentric components.Antialiasing: We use the fwidth() function (standard in GLSL 4.1) to measure the rate of change of the coordinates relative to screen pixels. This allows us to draw a line that is exactly $N$ pixels wide.OpenGL Shading Language// GLSL Fragment Shader for Barycentric Wireframe
in vec3 vBarycentric;
uniform float uLineWidth;
uniform vec3 uLineColor;
uniform vec3 uFillColor;

float edgeFactor() {
    // fwidth gives the derivative (rate of change) in screen space
    vec3 d = fwidth(vBarycentric);
    // Smoothstep creates a soft antialiased edge based on line width
    vec3 a3 = smoothstep(vec3(0.0), d * uLineWidth, vBarycentric);
    // The edge factor is the minimum distance to any of the 3 edges
    return min(min(a3.x, a3.y), a3.z);
}

void main() {
    float mixVal = 1.0 - edgeFactor();
    // Mix wireframe color with shaded color based on edge factor
    // Result is a perfectly smooth line on top of the mesh
    finalColor = mix(shadedColor, uLineColor, mixVal);
}
5.4 Reverse-Z Depth BufferingCAD scenes vary wildly in scale, from sub-millimeter watch gears to 100-meter ships. The standard floating-point depth buffer loses precision at far distances, causing z-fighting (flickering) on distant coplanar surfaces.Implementation Strategy: Switch the entire engine to Reverse-Z.44Clip Control: Set clip space conventions to $$ using glClipControl (available in OpenGL 4.5 or via extension in 4.1). If strictly 4.1 without extensions, we must manually adjust the projection matrix.Depth Clear: Clear Depth Buffer to 0.0 (instead of the default 1.0).Depth Function: Set glDepthFunc(GL_GREATER).Projection Matrix: Map the Near Plane to 1.0 and the Far Plane to 0.0.This leverages the distribution of floating-point numbers (which have more precision near 0) to give vastly superior depth resolution to distant objects, virtually eliminating z-fighting in large engineering assemblies.446. Integrating with Open CASCADE (OCCT)OCCT provides the AIS (Application Interactive Services) layer, which manages the high-level presentation of objects. To implement the modern features described above, we must customize standard OCCT classes.6.1 Customizing AIS_ShapeWe cannot rely on the default AIS_Shape rendering, which uses the fixed-function pipeline or basic Phong shaders.GLSL Injection: We must use Graphic3d_ShaderProgram to attach our custom PBR vertex and fragment shaders to the Graphic3d_Aspects of the AIS_Shape.46Material Definition: OCCT 7.x introduced Graphic3d_PBRMaterial. We must populate this structure with data from our material library (e.g., Metallic, Roughness, IOR) and ensure these values are passed to our custom shader as Uniforms.48Texture Management: OCCT's Graphic3d_TextureMap class must be used to load Albedo, Normal, and ORM textures and bind them to the correct texture units expected by our GLSL shader.6.2 Extracting Geometry for High-Performance RenderingFor advanced use cases where OCCT's internal renderer is too limiting (e.g., implementing the Barycentric wireframe which requires vertex attribute modification), we must extract the raw geometry.Topology Exploration: Use TopExp_Explorer to iterate over all TopoDS_Face entities in the shape.Triangulation Access: Use BRep_Tool::Triangulation() to retrieve the Poly_Triangulation handle. This gives access to the raw array of nodes (vertices) and triangles.49Edge Tessellation: Use BRep_Tool::PolygonOnTriangulation() to get the indices of the face nodes that form the boundary edges. Crucial Insight: In CAD, we only want to render the boundary edges (the topological feature lines), not the internal diagonals of the tessellated triangles. By extracting only the indices returned by PolygonOnTriangulation, we can build a specialized Index Buffer (IBO) that draws only the clean wireframe outlines.347. UX and Workflow: Materials, Environment, and Tone MappingThe rendering engine is useless without a UI that facilitates the engineering workflow.7.1 Tone Mapping StrategiesThe choice of tone mapper dictates the color fidelity of the final image.The "Notorious Six" Problem: Standard tone mappers often skew highly saturated colors (Cyan, Magenta, Yellow) towards white or primary colors as they get brighter.Khronos PBR Neutral: This mapper preserves the hue of the base color. It is essential for CAD. If an engineer picks a "Safety Orange" material, it must look like Safety Orange, not a desaturated yellow-orange.21Implementation: The tone mapping pass should happen in a final "Post-Process" shader (rendering a full-screen quad) after the PBR lighting calculation but before the UI is drawn. This shader also handles Gamma Correction (linear to sRGB conversion).7.2 Material Library OrganizationFollowing Shapr3D's example, the material library should be organized by Physical Category (Metal, Plastic, Wood, Glass) rather than optical properties.Presets: Pre-define standard engineering materials (e.g., "Aluminum 6061 T6", "ABS Plastic", "Polycarbonate").Drag-and-Drop: Implement a UI (using Qt Quick or Widgets) that allows dragging a material icon onto a specific TopoDS_Face or the entire TopoDS_Solid.7.3 Environment ControlsSince PBR relies on IBL, the lighting is the environment. The UI must allow:Rotation: A slider to rotate the environment cubemap. This allows the engineer to move the specular highlights to inspect surface curvature (zebra stripe effect) without moving the camera.Exposure: A slider to control the intensity of the light, critical for checking how the model looks in dim vs. bright conditions.8. Strategic RecommendationsBased on the deep analysis of current technology and industry benchmarks:Prioritize Khronos PBR Neutral Tone Mapping: Adopt this as the default for the modeling viewport to ensure color confidence.Use Weighted Blended OIT: Abandon depth peeling. The performance cost is too high for complex CAD assemblies, and the visual approximation of Weighted Blended OIT is sufficient for engineering visualization.Invest in High-Quality HDRIs: Bundle the application with high-dynamic-range studio lighting environments (like those from Poly Haven). The quality of the PBR render is mathematically bound to the quality of the light source.Hybrid Viewport Modes: Implement a clear toggle between "Engineering View" (Matcap, Cavity Shader, Wireframe) for geometry editing and "Visualization View" (PBR, Shadows, Tone Mapping) for design review. Do not try to make one view do both.9. ConclusionImplementing a realistic material rendering system for CAD is a complex convergence of physics, mathematics, and software engineering. It requires moving beyond the legacy fixed-function pipeline of OpenGL into a modern, programmable, physically based workflow. By leveraging the geometric precision of Open CASCADE, the robust UI framework of Qt 6, and the advanced shading capabilities of OpenGL 4.1 Core Profile, developers can create a visualization environment that not only depicts the shape of an object but communicates its substance, finish, and intent. The roadmap provided herein prioritizes engineering accuracy (Reverse-Z, Neutral Tone Mapping, Accurate Edges) over pure cinematic flair, aligning with the core requirements of the professional design industry.10. Detailed Implementation Plan (Step-by-Step)Phase 1: Core Rendering ArchitectureSetup Qt 6 & OpenGL 4.1 Context: Create QOpenGLWidget subclass. Enforce QSurfaceFormat to Version 4.1, Core Profile.OCCT Bridge: Initialize OpenGl_GraphicDriver with the Qt context. Create V3d_View and V3d_Viewer.Render Loop: Implement the separate render thread and signal/slot mechanism for frame updates.Reverse-Z: Implement infinite far plane projection matrix and GL_GREATER depth test.Phase 2: PBR Shader DevelopmentAsset Loading: Implement stb_image or Qt Image loading for HDRI (.exr / .hdr).IBL Pre-processing: Write a standalone tool (or use cmgen) to generate Irradiance (.ktx) and Prefiltered Specular maps from HDRIs.Uber-Shader: Write the GLSL PBR shader implementing Cook-Torrance, GGX, Smith-Schlick, and IBL sampling.Material System: Create a C++ struct mapping glTF 2.0 parameters (Albedo, Metallic, Roughness, AO) to Uniform Blocks (UBO).Phase 3: Visualization & QualityTone Mapping: Implement Khronos PBR Neutral and Gamma Correction (to sRGB) in the final fragment stage.Edge Rendering: Implement the Barycentric wireframe shader. Modify OCCT data extraction to inject barycentric attributes into the VBOs.Transparency: Implement Weighted Blended OIT (2-pass rendering with accumulation framebuffers).Phase 4: UI & Interaction (Qt)Material Library: Create a Qt UI panel to drag-and-drop materials.Environment Controls: Add sliders for HDRI rotation and intensity.Viewport Modes: Implement the state machine to toggle between "Engineering" (Matcap/Edges) and "Visualization" (PBR) modes.End of Report.




---

# ChatGPT Research


Executive Summary
OneCAD’s rendering strategy will marry modern Physically Based Rendering (PBR) techniques with the unique demands of a CAD modeling environment. The goal is to deliver realistic material appearance (via a Cook–Torrance microfacet shader with image-based lighting) while preserving the crispness and precision expected in CAD viewports (e.g. visible silhouette edges and high depth precision). The OneCAD application will use on-demand rendering – updating only on user interaction – rather than a continuous game-like loop. By default, models will be shown in a “Shaded + Edges” style, with progressive refinement of geometry and visual quality after interactions. This report details the requirements from OneCAD’s spec, surveys relevant rendering techniques (PBR, normal mapping, tone mapping, shadows, screen-space effects, transparency, edge rendering), and analyzes how leading tools (Blender, Shapr3D, Fusion 360, SolidWorks) implement these features. We then propose an implementation blueprint for OneCAD’s rendering, a phased roadmap, and a build-vs-buy evaluation of using existing rendering engines. In summary, OneCAD should implement a robust baseline (OpenGL/Metal based PBR shader with edge overlays, on-demand redraw, Reverse-Z buffering) and incrementally add features like HDR environment lighting and optional ray-traced renderings, learning from modern CAD and graphics engines. This approach will ensure visually compelling output (for presentation-quality screenshots/renders) without compromising interactive performance for modeling.
Requirements from OneCAD Specifications (Rendering Constraints)
OneCAD’s internal specification outlines clear rendering requirements aimed at balancing visual quality with performance:
On-Demand Render Loop: The viewport must render frames only in response to user actions or navigation events, not run at 60 FPS continuously. This saves energy and ensures the CAD modeler’s CPU/GPU are free when the scene is idle. A flowchart in the spec shows an idle-event-update-render cycle with an inertia loop for continued motion if needed.
Default Display Modes: The standard viewing style is “Shaded + Edges” (solid shaded surfaces with an overlay of their sharp edges). The spec lists this as the default mode (✅), with options for “Shaded” (surfaces only) or “Wireframe” (edges only) as alternatives. This means OneCAD must support drawing face geometry with lighting, and overlaying edge lines for clarity. It implies managing depth sorting or offsets so edges appear on top without z-fighting.
Progressive Tessellation: To give immediate feedback after geometry operations, OneCAD will first display a coarse mesh then refine it in the background. The spec defines Coarse (~1.0 mm linear tolerance) for instant feedback, Medium (~0.1 mm) for normal view, and Fine (~0.01 mm) for high quality. A flowchart illustrates showing the coarse mesh on operation complete, computing the fine mesh on a background thread, then swapping it in. OneCAD’s rendering must accommodate dynamically updating mesh LODs.
Dynamic Quality During Navigation: While the user is actively orbiting, panning, or zooming, the renderer should temporarily drop to lower quality to maintain interactivity. Specifically, the spec says to disable MSAA, use the coarse tessellation, and turn off expensive effects (shadows, reflections) during camera motion. Once motion stops, high-quality settings are restored and a final redraw produced. This requires the render loop to be aware of interaction states and quality toggles, ensuring a smooth experience with no hitching.
Depth Precision (Reverse-Z): OneCAD must handle very large models (e.g. buildings) without depth fighting artifacts when zoomed into small details. The spec mandates using a Reverse-Z depth buffer for improved precision. In practice, this means configuring the graphics API to use a reversed depth range (1 at near, 0 at far) with a floating-point depth buffer, which gives more precision for distant geometry. The spec even notes an OCCT flag view->ChangeRenderingParams().ToReverseDepth = Standard_True; to enable this. OneCAD should thus initialize the rendering context (whether OpenGL or Metal) for reverse-Z and use a floating depth format, greatly reducing Z-fighting for scenarios like “100 m building with 2 mm screws”.
In summary, OneCAD’s spec calls for an event-driven renderer (no constant redraw), default shaded+edged visualization, multi-LOD tessellation, adaptive quality during view changes, and a reverse-Z depth technique for robustness. These will serve as design constraints for the rendering system implementation. By adhering to them, OneCAD can ensure high performance on macOS (Apple Silicon GPUs) and a clean CAD viewing experience by default.
PBR and Rendering Techniques for CAD Visualization
Modern physically-based rendering techniques can significantly enhance visual realism of CAD models. However, a CAD application must balance visual fidelity with performance and clarity. Below, we review relevant techniques – explaining their visual impact, performance cost, pitfalls, and recommended usage in OneCAD’s context (Must-Have ✅ vs Nice-to-Have ✳ vs Skip 🚫).
Microfacet BRDF (Cook–Torrance with GGX & Schlick Fresnel)
A physically based shading model is essential for realistic materials. The Cook–Torrance microfacet BRDF uses a statistical model of surface micro-roughness to compute specular reflections more accurately than Phong/Blinn models. Most PBR implementations (including glTF 2.0 and Unreal/Unity) use this model with specific components: a GGX (Trowbridge-Reitz) normal distribution for microfacets, the Smith GGX geometry function for masking-shadowing, and the Schlick Fresnel approximation for viewing-angle-dependent reflectivity
google.github.io
google.github.io
. This combination produces realistic highlights that get broader and dimmer as roughness increases, and simulates the increase in reflectivity at grazing angles (Fresnel effect). The GGX distribution is known for its long tail and is widely adopted for its balance of realism and performance
google.github.io
. Cost & Pitfalls: A Cook–Torrance shader is slightly more expensive than a legacy Phong shader – it involves a few more math operations (several dot products, a pow for Fresnel, etc.) per fragment. However, it remains real-time friendly, especially on modern GPUs, and is practically required for material realism. One must ensure energy conservation (not double-counting diffuse and specular energy); using the Schlick Fresnel and a Lambertian diffuse by default helps maintain plausible results
google.github.io
google.github.io
. A pitfall to avoid is the darkening of very rough metals due to single-scatter energy loss – some renderers apply a multi-scatter energy compensation for rough surfaces
google.github.io
google.github.io
, though this can be skipped initially for simplicity. Recommendation: ✅ Must-Have. Implement a Cook–Torrance BRDF in OneCAD’s shader. It yields a consistent, physically-grounded material response for metals and dielectrics. By adopting a standard microfacet model, OneCAD will ensure its materials (e.g. if imported via glTF) look correct. Using GGX+Smith and Schlick is a proven approach
google.github.io
 that balances realism with performance, suitable for real-time CAD viewport rendering.
Metallic-Roughness Material Workflow (glTF 2.0 Convention)
The metallic-roughness workflow is a PBR material parametrization used by glTF 2.0 and modern engines. In this model, surfaces are described by a base color (albedo), a metalness value, and a roughness value. Metallic surfaces use the base color for specular reflection tinting, whereas dielectrics use base color for diffuse only (with a fixed specular reflectance around 4%). Roughness controls the microsurface scatter – low roughness yields sharp, mirror-like reflections; high roughness gives broad, dull highlights. glTF’s PBR spec is explicitly based on this model
registry.khronos.org
, enabling consistent material interpretation across tools. OneCAD should design its material system around these parameters for interoperability. glTF defines the parameter ranges and texture channel packing: e.g. an optional metallicRoughness texture uses blue channel for metalness and green for roughness
registry.khronos.org
. Base color textures are sRGB, roughness/metal are linear
registry.khronos.org
registry.khronos.org
. Following these conventions makes it straightforward to import/export materials to glTF (and by extension, between OneCAD and other content pipelines). Cost & Pitfalls: The metal/rough model is fairly cheap to evaluate – it’s essentially feeding the BRDF different F₀ values and roughness into the microfacet equations. The main “cost” is texture lookups if maps are used (e.g. up to three maps: albedo, normal, and a combined ORM – occlusion/roughness/metal – map). This is usually fine even on integrated GPUs. A pitfall is that artists need to understand the somewhat unintuitive metalness parameter (since it’s binary for real-world materials: either 0 or 1 for most things). However, using glTF’s approach (with default values and optional textures) is straightforward. Recommendation: ✅ Must-Have. OneCAD should adopt a Principled PBR material with BaseColor + Metallic + Roughness as core properties. This aligns with glTF 2.0’s standard
registry.khronos.org
 and ensures OneCAD can leverage existing material libraries. We will use sensible defaults (e.g. metalness 0 for plastics, 1 for metals, roughness 0.5 if unspecified) and perhaps expose a simple UI (color picker for base color, sliders for metal/rough). This gives users an intuitive yet powerful way to get realistic materials without dealing with overly complex parameters (like specular glossiness, which we will avoid unless needed via extension).
Image-Based Lighting (IBL: Environment Maps, Irradiance, and Reflections)
Image-Based Lighting uses environment images (HDR panoramas) to light objects realistically. It involves two main components: diffuse irradiance and specular reflections from the environment. A common technique (used in Unreal Engine, Filament, etc.) is the split-sum approximation for specular IBL
cdn2.unrealengine.com
cdn2.unrealengine.com
: one pre-computes a prefiltered environment map (a mipmapped cube map where each mip level is blurred to the appropriate roughness) and a BRDF integration LUT (a 2D lookup for the specular Fresnel/geometry term). Meanwhile, diffuse lighting uses an irradiance map – a heavily blurred (Lambertian convolution) version of the environment. At runtime, the shader samples the irradiance map for diffuse ambient light, and samples the prefiltered specular map with the material’s roughness (mip level) and view-dependent reflection vector, modulated by the BRDF LUT for Fresnel and visibility
cdn2.unrealengine.com
cdn2.unrealengine.com
. For OneCAD, IBL means that a model will look “grounded” in a lighting environment: metals will show realistic reflections of the surroundings, and all objects get subtle color bleed from the environment. This is far more realistic than defaulting to simplistic directional lights. We can provide a set of HDRI environment presets (studio light, outdoor sunny, soft indoor, etc.) and allow users to rotate the environment for best appearance. Cost & Pitfalls: IBL can be expensive memory-wise (cubemaps and mipmaps) and moderately expensive in shader ALU (sampling multiple LODs of cubemaps and using the LUT). However, these computations are done per fragment and are well within real-time budgets on modern GPUs, especially if we limit environment resolution (e.g. 512³ cube) and maybe sample only one specular MIP level (since the LUT accounts for the rest). A major pitfall is managing dynamic range correctly – environment maps are HDR, and specular reflections can easily bloom or cause aliasing if not tone-mapped (addressed separately in tone mapping). Also, parallax issues: IBL assumes distant environment; for CAD (usually models are standalone) this is fine. Finally, if the user expects shadows from the environment (e.g. sun position), a simple IBL doesn’t cast ground shadows unless we add techniques like shadowcatcher ground planes (Shapr3D does provide an optional ground plane that receives shadows
support.shapr3d.com
). Recommendation: ✅ Must-Have (v1.5). IBL should be introduced as soon as basic shading is in place. It dramatically improves visual quality for relatively low cost. In OneCAD v1.0, we might start with a simple neutral ambient light, but by v1.5, implementing environment map lighting with a set of preset HDRIs and a precomputed convolution pipeline (ideally run at startup or pre-shipped) is highly recommended. This will give OneCAD a modern look on par with other tools. We will include a BRDF LUT and either generate or ship prefiltered environment textures
cdn2.unrealengine.com
. Users can then select different lighting setups quickly.
Normal Mapping (Tangent-Space Detail Maps)
Normal mapping adds fine surface detail (bumps, grooves, patterns) without increasing mesh complexity, by perturbing the surface normals per-fragment. For CAD, this can be useful to simulate textures like knurling on a knob, leather grain, or cast/textured surfaces that are modeled as smooth geometry but should appear bumpy. Normal maps are typically authored in tangent space, requiring a consistent tangent basis on the mesh. A standard approach is Mikkelsen’s Tangent Space (MikkTSpace), which most tools use to ensure the normal maps show correctly regardless of modeling software
github.com
. OneCAD can generate tangents for imported mesh data (OCCT’s tessellator can provide normals, and we can compute tangents aligned to UVs). Using MikkTSpace means OneCAD’s normal maps will align with normals baked in tools like Blender or Substance (ensuring no seams or odd shading). Cost & Pitfalls: Sampling a normal map is one extra texture lookup and some math to transform it to world/viewport space – very cheap on modern hardware. The bigger challenge is that CAD models often don’t have UV maps by default (OCCT tessellation doesn’t include UVs unless the surface has a natural parametric UV space). We may need to generate procedural UVs or planar projections for applying seamless textures. Pitfalls include seams if tangents are inconsistent, or incorrect normal map format usage (normal maps must be sampled as linear data, not sRGB). Also, if a user expects to apply a normal map, they need some UI to scale and orient it – this can be tricky for arbitrary CAD parts. However, for imported models or when we support painting textures, the groundwork is needed. Recommendation: ✳ Nice-to-Have (v2). Normal mapping is not critical for a v1 CAD modeler focusing on shape design; most CAD users care more about geometry edges and accurate shading than tiny surface relief. However, as OneCAD matures toward visualization features (photo-realistic rendering of the designed part), supporting normal maps (and possibly roughness maps) will become important. We should plan the rendering engine to be ready for normal maps (include tangent calculation in mesh pipeline). If using glTF materials, we should parse and store normal maps for imported assets. In the short term, we might skip UI for custom normal maps, but ensure the shader and data structures can handle it later.
Tone Mapping and Color Management (HDR to LDR, Linear Workflow)
A linear lighting workflow is mandatory for physically based rendering – all lighting calculations should happen in linear space, and only at the end be converted to the display colorspace (sRGB). Tone mapping refers to compressing a high dynamic range (HDR) image (result of PBR shading with bright reflections, etc.) into the low dynamic range of a typical display in a visually pleasing way. Simply clamping or linearly scaling leads to washed-out or overly contrasted images. Thus, engines use curves like ACES, Reinhard, Filmic, etc. Tone mapping ensures highlights roll off naturally and dark areas retain detail
dev.epicgames.com
danthree.studio
. For example, ACES is a filmic curve that is now common in many renderers; Blender uses a Filmic curve by default for its viewport. OneCAD should at minimum apply a gamma correction (to sRGB) after rendering. But ideally, we implement a simple tone map (Filmic or ACES) to handle very bright reflections and avoid burn-out. The Blender Filmic transform is an easy starting point – it’s essentially a gentle S-curve. More advanced: ACES (Academy Color Encoding System) is an industry standard for consistent color pipeline, though full ACES may be overkill for real-time CAD (it’s more for matching across mediums). The key is that our renderer will produce HDR intermediates (especially with bright metal reflections or emissive highlights), and these must be brought into 0–255 range gracefully
dev.epicgames.com
. Additionally, we must manage color spaces: any color texture (albedo map) should be decoded as sRGB and converted to linear for lighting
registry.khronos.org
; our rendered frame, after tone mapping, should be encoded to sRGB for display (unless on HDR display, which we can defer for now). Cost & Pitfalls: Tone mapping adds a post-processing pass (or can be integrated in the main shader if using gamma only). A simple algorithm (like Reinhard) is a few ALU ops per pixel – trivial cost. A more advanced like ACES is still just a curve evaluation (could be done via a small LUT or analytic approximation). The cost is negligible compared to lighting. The pitfalls are in calibration – making sure the output looks good across devices. Also, the user might want control (some CAD users may prefer an un-tonemapped, “raw” look for technical accuracy of colors). Perhaps we provide an option to toggle tone mapping on/off. Another consideration is background – if the user has a white UI background and we tone map to a grayish sky, it might look off. We might allow a solid color background in addition to environment images for clarity in technical illustrations. Recommendation: ✅ Must-Have (basic gamma now; full tone map by v1.5). In v1.0, we will at least ensure correct gamma: render linear, output sRGB. By v1.5 (when HDR environment lighting and bright reflections come in), we should implement a Filmic tone map to enhance visual appeal. This will make OneCAD screenshots look professional. We will also handle color management by using linear blending for lighting and only doing gamma correction at the end. If possible, adopting Filmic (which Blender uses to avoid harsh clipping) can be a quick win: it yields natural-looking results where highlights don’t clip abruptly. As an example definition: “Tone mapping translates scene-referred HDR renders into the display-referred range (sRGB) so that contrasts and highlights appear natural”
danthree.studio
 – this is exactly what we need for OneCAD’s final framebuffer.
Shadows (Direct Shadows and Contact Shadows)
Shadows greatly enhance depth perception and realism in a scene. In OneCAD’s context, shadows would primarily come from either a main directional light (sunlight in an environment) or from environment IBL (which typically is ambient and directionless, though some HDRIs have a strong sun component). Implementing real-time shadows usually means using shadow maps for point or directional lights. For example, a simple approach: if using an HDR environment with a visible sun, we could cast a parallel light and generate a shadow map for it (cascaded shadow maps for large scenes if needed). Shadows on the model itself help the user discern spatial relationships (e.g. a bolt casting a shadow on a plate). Shapr3D’s Visualization offers a ground plane shadow option (an infinite ground that catches model shadows)
support.shapr3d.com
, which is great for presentation. OneCAD could do similarly – an option to enable a ground plane that receives shadows from a key light (and perhaps reflections). Additionally, contact shadows (a.k.a. screen-space shadows) can be used to augment standard shadows. These are small-scale shadows directly under objects, often implemented by screen-space raymarching of depth (similar to SSAO but directional). Engines like Unity HDRP and Blender Eevee use “contact shadows” to capture object self-shadowing that is missed by low-res shadow maps
docs.unity3d.com
threejs.org
. Essentially, the GPU traces a short ray in screen space from each pixel toward the light and checks for occlusion in the depth buffer
panoskarabelas.com
. This can fill in the gap, for example, under a screw head where a global shadow map might not have enough resolution. Cost & Pitfalls: Regular shadow maps are moderately expensive (rendering the scene from the light’s POV); quality issues like aliasing and acne must be managed (PCF filtering, bias). Cascaded shadows for a large model could be complex; however, many CAD scenes are medium scale and a single shadow map might suffice. Contact shadows (screen-space) are cheaper than increasing shadow resolution, but they have limited range and only render what’s in view
panoskarabelas.com
. They can also introduce noise or artifacts at screen edges. For OneCAD, implementing a full cascaded shadow system might be overkill initially; a single light + single shadow map (with maybe 2K resolution) could do for small-to-medium scenes. Ambient occlusion (next section) can also substitute for some sense of contact shadow. Recommendation: ✳ Nice-to-Have (v1.5) for basic shadows, 🚫 Skip for advanced contact shadows initially. For OneCAD v1.0, we might not have any shadows (just ambient lighting), focusing on edges and AO for depth. By v1.5, introducing an optional shadow (e.g. a toggle for “shadow from main light”) would significantly enhance realism especially when using environment maps with a clear light source. We should implement a basic shadow map for a directional light (perhaps assumed from HDR environment’s brightest direction). We’ll also add a ground plane shadow catcher for nicer screenshots. Contact shadows or ray-traced shadows can be skipped at first; if users demand more shadow detail, we can revisit screen-space shadows. CAD users often value some shadowing (for context) but may prefer to keep it subtle to not obscure model details. So our focus: a simple, soft shadow implementation that runs fast.
Ambient Occlusion (SSAO)
Screen-Space Ambient Occlusion (SSAO) is a real-time technique to approximate occlusion of ambient light in concave areas of a scene. It darkens creases, holes, and contact areas between objects, giving a subtle depth cue beyond what direct lighting provides. Ambient Occlusion in CAD models can help visually distinguish parts and convey proximity. For example, the corner of an assembly or the junction of a bolt and a plate will appear slightly shadowed, which looks more realistic. SolidWorks and other CAD software have an SSAO option in their RealView/graphics settings
support.hawkridgesys.com
 because it adds realism with minimal performance cost. SSAO works by sampling the depth buffer around each pixel and estimating how much of the hemisphere is occluded. It’s purely a screen-space post-process, meaning it can’t capture large-scale shadowing (that’s what actual shadows are for), but is great for small-scale shading. It operates even with environment lighting by attenuating ambient term in crevices
help.solidworks.com
. Cost & Pitfalls: SSAO is relatively cheap. Basic SSAO (Crytek-style or HBAO) might cost a few milliseconds on modern GPUs at 1080p. It involves multiple samples per pixel and a blur. The quality can vary – too strong AO looks like a dark halo around objects, too weak and it’s not noticeable. Also, since it’s screen-space, objects only occlude what’s visible; you might see AO “fade” at screen edges or if an object is off-screen. Still, many engines consider SSAO essential for depth. Another pitfall: it can conflict with technical visuals; for instance, in precise engineering drawings, AO is not physically accurate shadow, it’s an approximation – but since OneCAD is going for realistic rendering for makers, AO is acceptable. Recommendation: ✅ Must-Have (v1.5). SSAO is a quick win for improved visuals. We will include a toggleable SSAO post-process once the basic pipeline is up. Use a simple implementation (e.g. horizon-based ambient occlusion) and default it to a low-to-moderate strength so it doesn’t overpower edges. Many CAD users appreciate the effect: SolidWorks Ambient Occlusion “adds realism…objects appear more lifelike”
javelin-tech.com
. We’ll follow suit. It should be optional (for users who want a completely flat-lit look for screenshots). SSAO will complement shadows: where our shadow map might be low-res or absent, SSAO will still give some grounding of objects.
Reflections and Screen-Space Reflections (SSR)
Highly polished or reflective materials (chrome, mirror-like surfaces) are common in product design. While environment maps provide static reflections, they lack reflections of adjacent objects. For example, a chrome part should reflect a neighboring part. True mirror reflections require ray tracing or a screen-space hack. Screen-Space Reflections (SSR) is a technique that ray-marches in the screen buffer to find reflections of what is currently visible
lettier.github.io
. It creates the illusion of reflections by reusing rendered pixels: essentially “reflecting the screen onto itself”
lettier.github.io
. SSR can show inter-reflections between objects, adding realism (e.g. a shiny floor reflecting the machine sitting on it). However, SSR only works for what’s on screen – anything off-screen or obscured will not appear, often leading to reflections that suddenly disappear or cut off (common SSR artifacts). It’s also more of a nice touch than a necessity in CAD: CAD models often use environment reflections for a quick approximation, and precise inter-object reflections are usually not critical unless doing a final render. Cost & Pitfalls: SSR is one of the more expensive screen effects. It requires iterative ray steps per pixel, which can cost a few milliseconds. It’s also tricky to get stable (noise, hit-miss artifacts, need for blur). In CAD viewports, geometry is often complex (lots of small features) – SSR might produce noisy results or miss thin features. A big pitfall is user confusion: reflections might look incorrect when parts disappear out of frame (SSR can only reflect what the camera sees
doc.babylonjs.com
). Many engines (Unity, Unreal) allow SSR but with understanding of its limitations. Recommendation: 🚫 Skip (for now). SSR is probably overkill for OneCAD’s interactive viewport. We will rely on environment map reflections (which give a general believable reflection) and if true reflections are needed, we can offer a ray-traced render mode in the future (see roadmap for a path-trace preview). SSR could be a “nice-to-have” for certain demos (e.g. shiny car body reflecting wheels), but given the implementation complexity and artifact-prone nature, it’s not worth the development effort in early versions. If we ever integrate a real-time renderer like Unreal Engine’s pipeline or move to a fully ray-traced viewport, that would supersede SSR anyway. So we’ll skip SSR in OneCAD v1.x, focusing on solid IBL and potentially an offline renderer for perfect reflections.
Global Illumination and Screen-Space GI (SSGI)
Global Illumination (GI) refers to bounced light – color bleeding and indirect lighting between objects. In CAD scenes, GI is not usually a priority because models are often shown in isolation with environment lighting. Nonetheless, if you have multiple parts, one part could cast color onto another (a red part near a white part will tint it slightly). Full real-time GI is complex (requires ray tracing or light maps). Screen-Space Global Illumination (SSGI) is an approximation, similar to SSAO but for diffuse light bounce – it uses screen-space info to simulate one bounce of light. It’s even more of a hack than SSR; it can add a bit of light bleeding but suffers from the same off-screen limitations. For CAD, the effect of GI is subtle and typically handled acceptably by ambient light plus perhaps an artistic light rig. Cost & Pitfalls: SSGI is quite expensive and not widely used except in certain game engines as an experimental feature. It samples the screen depth/color to estimate indirect light. The results can be flickery and inaccurate (and like SSR, only considers on-screen contributors
forums.unrealengine.com
). True GI (like path tracing) is obviously even more expensive (not feasible in real-time without hardware raytracing, which we can’t assume universally, especially on integrated Apple GPUs without raytracing cores). The pitfall: implementing SSGI might not yield proportionate benefit – scenes could just look slightly muddier or unpredictably lit. Recommendation: 🚫 Skip. OneCAD should skip SSGI and GI for the real-time viewport. Instead, rely on environment lighting + ambient occlusion to approximate global light. For cases where a user wants photoreal lighting with bounces, we plan a path-traced render preview (offline or using a GPU raytracer) in a future version. SSGI is not mature enough to justify inclusion, and most CAD users won’t miss subtle color bleed. They’re more concerned with direct shadows and overall illumination which we handle via environment and possibly a direct light.
Transparency Rendering (Order-Independent Transparency)
Displaying transparent objects (glass, plastic covers, etc.) is challenging due to the need to blend fragments in correct depth order. Traditional alpha blending requires sorting objects from back-to-front which is complex for dynamic scenes with many transparent parts (and impossible to sort perfectly if transparents interpenetrate). Modern solutions include Order-Independent Transparency (OIT) techniques such as weighted blended OIT. For example, Weighted Blended OIT (by McGuire and Bavoil, 2013) avoids per-pixel sorting by accumulating color and coverage in a single pass and then doing a post composite
casual-effects.blogspot.com
casual-effects.blogspot.com
. This gives a plausible result without sorting, and can handle many overlapping layers of transparency (like multiple glass panels) without cracking. SolidWorks and others have adopted GPU-based OIT for their viewports to correctly render overlapping translucent parts in assemblies
amd.com
. In OneCAD, while many hobbyist models may not involve transparency, it’s still important for things like visualizing an assembly with a see-through housing, or applying an opacity to a reference object. Cost & Pitfalls: Basic alpha sorting could be implemented if there are few transparent objects, but it becomes untenable with lots of transparent triangles. OIT (especially weighted blended) has a moderate cost: it requires using multiple render targets and blending, but it’s designed to be efficient on modern GPUs
casual-effects.blogspot.com
. The quality is an approximation – e.g., colors are blended assuming a certain order bias. Pitfalls include slight darkening or color artifacts if weights aren’t tuned
casual-effects.blogspot.com
. Also, additive transparency (like colored glass filtering light) might not be perfectly captured, but the technique is generally “good enough” for real-time use. Another pitfall is that if MSAA is on, OIT can be more complicated to support (but we might disable MSAA when heavy transparency is present or use a shader approach to manual AA). Recommendation: ✳ Nice-to-Have (v2). For OneCAD v1.0, we might implement a simpler approach: enforce that entire objects (bodies) have a uniform transparency and sort those bodies. This could cover basic use (e.g., one transparent casing around other parts). Full per-pixel OIT can come later. By v2.0, if we want to robustly handle complex scenes with multiple transparent parts, implementing weighted blended OIT is the way to go – it’s in wide use, including in Cesium, Three.js, and CAD apps
casual-effects.blogspot.com
casual-effects.blogspot.com
. It avoids artifacts that manual sorting would produce (especially in CAD models where parts often intersect). The decision to postpone to v2 is to ensure we get the foundational shading and other effects right first. When we do implement it, we’ll leverage available research (the NVIDIA/JCGT paper on Weighted Blended OIT
casual-effects.blogspot.com
 gives us a ready blueprint). So, transparency will be supported in a basic form initially (some simple sorting or just allow user to ghost entire parts), and improved later.
Edge and Silhouette Rendering
Unlike pure entertainment rendering, CAD rendering benefits greatly from edge overlays to highlight shape features. Users expect to see sharp edges, feature lines, and silhouette outlines clearly, especially in “technical illustration” style views. OneCAD’s spec already highlights the default Shaded+Edges mode, and Shapr3D provides a “Visualized” mode that “enhances edge and silhouette visibility for better definition.”
support.shapr3d.com
. Edge rendering typically means drawing the model’s wireframe or a subset (only sharp edges above a certain dihedral angle, and silhouette edges – where the surface normal is perpendicular to the view direction). There are two main approaches: geometry-based edges or image-based (post-process) edge detection. Geometry-based is straightforward: use the mesh or CAD topology to get edge lines and draw them as lines on top of the shading. This requires some care with depth bias (to avoid z-fighting with the surfaces). For example, a common OpenGL trick is: draw filled polygons, then draw the wireframe with glPolygonOffset to raise it slightly
stackoverflow.com
, so the lines reliably appear on top without z-fight. Another trick is drawing edges in a second pass with depth test but using a slightly biased projection matrix. Image-based edge detection (like detecting discontinuities in the depth/normal buffer) can also produce silhouette outlines and has the advantage of catching silhouettes without explicitly having the edges, but it may miss small feature lines and can be less precise for CAD where exact edges matter. OneCAD can leverage OCCT’s topology: we know the edges of faces and which are sharp (via face normals). We can thus generate a list of edges to draw. Silhouette edges (which depend on view) could be identified on the fly or via GPU geometry shader, but since OCCT can give us per-face normals, identifying boundary and silhouette edges per frame is feasible if needed. Cost & Pitfalls: Drawing lines is very cheap compared to filled triangles. The cost is negligible, even if we draw every edge of a dense model. The pitfalls are visual: too many edges can clutter the image (especially on tessellated models where every triangle edge might draw if not filtered). So we should draw only sharp edges (edges between two faces meeting at > some angle, e.g. >30°) and boundary edges (outer silhouettes). Another pitfall is z-fighting – but we solve that with depth bias or polygon offset as mentioned. We should also consider line thickness and possibly a subtle color difference (e.g. dark grey lines on a light model). Some CAD packages also offer “hidden edges” dashed or in different style when an object is transparent or in certain modes
support.shapr3d.com
support.shapr3d.com
 (OneCAD spec has “Show Hidden Edges” toggle
support.shapr3d.com
). We can implement that later. Recommendation: ✅ Must-Have (v1.0). Edge rendering is critical for CAD clarity. From day one, OneCAD should support drawing edges and outlines on top of the shaded model. We will use polygon offset or a two-pass approach as the “standard trick” for hidden-line removal in OpenGL: draw polygons, then draw edges slightly offset
stackoverflow.com
. Initially, simply drawing all feature edges (and maybe silhouette edges via an algorithm) will suffice. As we refine, we can mimic Shapr3D’s approach: a “Visualize” mode with heavier outlines for silhouettes for presentations, versus a plain shaded mode for pure realism
support.shapr3d.com
. Nonetheless, for a beginner-friendly CAD, leaving edges on by default is advisable (Shapr3D clearly found it important enough to have an enhanced edges mode
support.shapr3d.com
). We will include preferences for edge color/thickness possibly, but the default will be a thin, subtle outline that doesn’t overpower the shading but is clearly visible.
Blender Rendering Pipelines: Eevee vs Cycles – Features and Materials
Blender provides two rendering engines that inform our analysis: Eevee, a real-time rasterization engine, and Cycles, a path-tracing engine. Understanding their capabilities helps us chart OneCAD’s own feature set and consider what level of realism vs interactivity is appropriate in a CAD context.
Feature Comparison Matrix
We summarize the key features of Eevee (realtime) and Cycles (path-tracer) and how they handle various aspects:
Feature	Blender Eevee (Real-time)	Blender Cycles (Path-trace)
Shading model	PBR (Principled BSDF shader) with approximations (no true GI)	PBR (Principled BSDF) with full global illumination
Lighting	HDRI Environment, Sun/Area lights (supports shadows via shadow maps)	All light types with physically accurate shadows/GI
Shadows	Yes – shadow maps (with softness, resolution settings), plus Contact Shadows toggle per light (screen-space raymarch for fine contact)	Yes – ray-traced shadows (accurate soft shadows based on light size)
Ambient Occlusion	Yes – SSAO option (approximate screen-space ambient occlusion)	Not needed (indirect lighting comes from path tracing, though can composite AO pass if desired)
Reflections	Yes – Screen-space reflections (SSR) and reflection probes; planar reflections for mirrors	Yes – accurate reflections via ray tracing (including self-reflections, multiple bounces)
Refractions	Yes – Approximate; requires screen-space, no true light bending	Yes – true refraction with correct caustics (if enabled; caustics are hard and often noisy)
Transparency sorting	Uses order-independent transparency via weighted blended technique (for Eevee’s materials marked “Alpha Blend/Hashed”)	Not applicable (Cycles naturally handles arbitrary transparency through path tracing)
Volumetrics	Yes – approximate (voxel-based)	Yes – true volumetric scattering (with noise)
Anti-aliasing	MSAA + TAA (Temporal AA) available for smooth output	Not real-time; outputs can be sampled to any quality (diminishing noise as samples increase)
Performance	Real-time (runs at interactive FPS, tuned for quick rendering)	Offline (seconds or minutes per frame for final quality; can do interactive viewport render but noisy until converged)
Relevance to OneCAD: OneCAD’s viewport is analogous to Eevee – it must be real-time and responsive, using similar tricks (shadow maps, SSAO, IBL, screen effects) to approximate realism. Cycles is akin to a “final render” mode (like what OneCAD might do in a future photoreal preview). The user expectation in CAD is that the normal modeling viewport is interactive but can be made quite nice (like Eevee), and if needed a separate high-fidelity render can be done (like Cycles, possibly via external renderer integration). Notably, both Eevee and Cycles share Blender’s material system – the Principled BSDF. This is an important lesson: Blender gives artists a unified material definition that works in both engines. In our case, we want OneCAD’s materials to be defined in a way that could feed either a real-time renderer or an offline one. So designing around a Principled-like model (which we are doing with metal-rough PBR) is validated by Blender’s example.
Blender’s Principled BSDF and Node-Based Material System
Blender’s Principled BSDF shader is essentially the Disney PBR shader that consolidates many physical material properties into a single node. It includes base color, metallic, specular, roughness, anisotropy, clearcoat, transmission, subsurface scattering, etc., in one package. The Blender Manual notes it’s “based on the Disney principled model also known as the ‘PBR’ shader.”
docs.blender.org
 In practice, this means artists can create almost any material (from plastic to metal to glass) by tweaking a few sliders, without building the shader from scratch. However, Blender’s material editing is node-based. The Principled BSDF is one node in a node graph – artists commonly connect image textures to the various inputs (albedo map to Base Color, roughness map to Roughness, etc.), combine nodes for effects, etc. This node system is extremely flexible (it can represent any layering or mapping), but it can be complex for new users or for CAD users who are not familiar with shader programming. CAD users typically expect a simpler UI (like choosing a material from a library, maybe adjusting a color or finish via sliders). Texture conventions: Blender (via Principled and glTF support) typically uses: Base Color in sRGB, Roughness/Metallic in linear (often combined in channels), Normal maps in tangent space (requiring proper UVs and normal map setting to non-color data). It also uses an Occlusion map (AO) as a multiply on ambient lighting if provided (glTF’s occlusion). These are exactly the conventions we plan for OneCAD, ensuring that if a user imports a glTF, the material will plug into our system one-to-one. Lessons for OneCAD UX: We want the power of a Principled shader without requiring our users to wire node graphs. For OneCAD, a likely approach is to expose a simplified material property panel: e.g. a dropdown or gallery of material presets (like “Aluminum, Glossy Red Paint, Glass, Rubber” etc. – Shapr3D provides 100+ preset materials in a library
support.shapr3d.com
), and some overrides like color tint or roughness slider. This corresponds to adjusting Principled BSDF parameters under the hood. Blender itself offers some presets (for example, the addon NodeWrangler can set up some common materials), but largely expects users to tweak nodes. OneCAD being beginner-focused should hide the nodes. Internally, we might implement materials similarly (a struct with those PBR parameters feeding our shader), but the UI will be form-based or pick-list-based, not a node editor. If a user needs complex texture mapping, that’s somewhat outside scope for v1 – but we can allow applying an image as “color map” easily enough, which is one node connection in Blender terms. Another relevant Blender feature is Color Management: Blender uses a Filmic view transform by default, which significantly improves the display of high dynamic range content (preventing harsh clipping). This has been one of the reasons Blender’s viewport looks good out of the box for renders. We likely should mimic this by including a filmic tone map. Blender also offers switching to standard sRGB if needed (for precise color work). For OneCAD, we could just bake in the filmic curve (no UI toggle initially), giving a nice result by default (since our target users are not doing linear color grading – they just want the model to look nice). Finally, Blender’s node-based workflow vs slider workflow: Node graphs allow extremely sophisticated materials (e.g. blending two BSDFs for layered materials, driving roughness with procedural textures, etc.). In CAD, such complexity is rarely needed – users use materials for visual feedback and maybe simple renderings, not elaborate VFX. So we conclude that providing a fixed set of material parameters (like Principled) with a simple UI is the right approach for OneCAD. If a user truly needs a custom material beyond that, they likely will export to a dedicated rendering software. Our goal is to cover 95% of use cases (which a principled model does) with simplicity. In summary, Blender teaches us to use a unified physically-based shader, keep the workflow consistent between real-time and high-quality modes, and to manage color/tone so the output is visually pleasing. We aim for OneCAD’s viewport to achieve near-Eevee quality (real-time PBR with IBL, shadows, SSAO, etc.) and leave room for a Cycles-like pathtrace for final renders if needed.
Shapr3D’s Visualization Pipeline and User Experience
Shapr3D is a modern CAD tool known for its smooth real-time visuals, especially on iPad and Mac. It provides a useful benchmark for OneCAD both in terms of technical features and user workflow integration (modeling vs visualization). We analyze Shapr3D’s approach:
Visualization Features in Shapr3D
Shapr3D essentially has two display modes: the normal modeling viewport (with solid colors, edges, etc.) and a dedicated Visualization mode for rendering-like quality. Key features of its visualization:
Physically-Based Materials: Shapr3D offers a library of over 100 materials in Visualization mode
support.shapr3d.com
. These include metals, plastics, glass, etc., likely using a PBR model under the hood. The user can drag-and-drop these onto parts or faces
support.shapr3d.com
support.shapr3d.com
. Each material has properties; for example, some allow color adjustment or roughness tweaks
support.shapr3d.com
. This indicates Shapr3D uses something akin to the Principled shader (for instance, a wood material might have a texture, a metal might allow changing its color tint or roughness).
Environment Lighting: Shapr3D provides several environment options (HDRI backgrounds) and allows customization
support.shapr3d.com
support.shapr3d.com
. The user can pick an environment (studio, outdoor, etc.), rotate it, and adjust its brightness. It also has a Ground Plane feature – an infinite ground that receives shadows, with options to snap it to the model’s bottom or a plane
support.shapr3d.com
. This gives context and a shadow anchor for the model in the environment.
Shadows: In Visualization mode, shadows are rendered. Shapr3D likely uses the environment’s key light or a default light to cast shadows. They even allow turning shadows off
discourse.shapr3d.com
 (some users in the community wanted to remove ground shadows for clear product images). This suggests a toggle. The ground plane specifically receives shadows
support.shapr3d.com
. The quality of shadows in Shapr3D is quite good (soft and not too jaggy in examples), implying high-resolution shadow maps or filtered shadows.
Ambient Occlusion: Shapr3D’s documentation doesn’t explicitly mention SSAO, but users on forums have asked for ambient occlusion to be added for contact shadows
discourse.shapr3d.com
. It’s unclear if Shapr3D added SSAO; possibly they rely on shadows + high-quality lighting to produce a similar effect. It’s an area OneCAD could even improve upon if Shapr3D lacks it.
Outlines and Edges: In normal modeling, Shapr3D uses a “Visualized” shader mode that shows edges and silhouettes
support.shapr3d.com
. In Visualization mode, however, the goal is more realism – likely edges are not drawn (it’s like a render). Indeed, their help calls it “real-time renders of your models”
support.shapr3d.com
, which implies no black outlines in that mode. So Shapr3D chooses: modeling mode = edged, visualization mode = pure render. They also have a surface analysis mode with zebra stripes and curvature, but that’s more for modeling checks
support.shapr3d.com
 (OneCAD might implement those in modeling view as well, but not relevant to general rendering).
Depth of Field: Shapr3D’s Visualization allows enabling Depth of Field with camera settings
support.shapr3d.com
. The user can adjust aperture/focus to blur background parts, which is great for presentation (photographic look). This is a purely aesthetic feature but an important one for “visual storytelling” when presenting a design. It’s not typical in modeling view, only in visualization mode.
Background/Environment styles: The environment in Shapr3D can be visible (showing the HDRI) or possibly set to a solid color/backdrop. In their AR mode, they obviously use device camera as background. For normal use, a neutral background or the environment blurred could be what they do. OneCAD should allow a plain background too (some prefer plain white or gradient for technical presentations).
Anti-aliasing and resolution: On iPad, Shapr3D likely uses MSAA 4x or similar plus possibly TAA in Visualization for smooth output. OneCAD on Mac can leverage 4x MSAA as specified. High quality AA is expected in still visualization.
The table below compares Shapr3D’s features with Fusion 360 and SolidWorks (as requested, for context):
Feature / Aspect	Shapr3D (Modeling vs Visualization)	Fusion 360	SolidWorks (RealView)
Materials & Library	~100 PBR materials, drag onto bodies/faces
support.shapr3d.com
support.shapr3d.com
; basic color/scale edits.	Many preset materials (appearance library) with adjustable parameters; apply to bodies/faces.	Extensive material library (Appearances); apply to bodies/faces/features.
Lighting	IBL environments (multiple HDR presets)
support.shapr3d.com
; adjustable rotation/brightness; optional ground plane for shadows
support.shapr3d.com
.	IBL environment (several presets, or custom HDR); one fixed key light for modeling view; ground shadows toggle.	RealView: uses environment maps for reflections and a fixed light; shadows toggle; ambient light + optional floor shadows.
Edges in viewport	Modeling: “Visualized” mode draws edges & silhouettes clearly
support.shapr3d.com
. Visualization: no edges (realistic render).	Modeling: has visual styles (Shaded, Shaded with Visible Edges, Wireframe, etc.) – typically uses “Shaded with edges” for modeling. Render workspace: no edges.	Modeling: can do Shaded with Edges or without; also “Cartoon” mode for outlines
support.hawkridgesys.com
. Photoview/Visualize renders: no drawn edges (unless stylized mode).
Shadows	Visualization: yes, dynamic shadows (soft) from environment/ground
support.shapr3d.com
; toggleable
discourse.shapr3d.com
. Modeling: no shadows (for performance).	Modeling: optional ground shadow and object shadow in viewport. Render: ray-traced shadows.	RealView: optional shadows in shaded mode (via shadow maps); also ambient shadows via AO
amd.com
.
Ambient Occlusion	Not explicitly mentioned; likely minimal or only in Visualization if at all. (User forum requests suggest it wasn’t initially present).	Yes, in viewport there is an ambient occlusion toggle (Fusion calls it “contact shadows” or similar, provides SSAO in viewport).	Yes, Ambient Occlusion toggle in RealView (approximate SSAO)
support.hawkridgesys.com
amd.com
.
Reflections	Yes, via IBL. Shiny materials reflect environment. No SSR in viewport.	Yes, environment reflections in viewport. (Fusion can also do SSR in its Advanced mode for the ground reflection plane).	Yes, environment reflections with RealView (need supported GPU); can see environment cubemap on shiny parts.
Transparency Handling	Likely order-independent (Visualization mode can show many transparent objects correctly blended). Modeling mode probably simpler (less transparency use).	Uses OIT in viewport for correct transparency layering.	Uses Order Independent Transparency (since SW2014) for viewport
amd.com
.
Depth of Field	Yes, in Visualization (with camera settings for focus/blur).	In Render mode (raytrace) yes; not in modeling viewport.	In Photoview/Visualize (raytrace) yes; not in normal viewport.
AR Mode	Yes, on iPad (USDZ export and AR view)
support.shapr3d.com
.	Yes, via cloud or local rendering to AR (Fusion has limited AR preview on mobile).	No integrated AR (but SolidWorks models can be exported to AR apps via e.g. eDrawings).
(Comparison notes: Fusion 360’s viewport is quite similar to Shapr3D’s: it uses real-time PBR with environment lighting, shadows, and a material library. SolidWorks RealView (with approved GPUs) gives environment reflections, SSAO and shadows, and even a “cartoon edges” mode
support.hawkridgesys.com
. Both Fusion and SW have separate photorealistic renderers for final output, like Shapr3D’s Visualization mode provides.)
Shapr3D UX Flow (Modeling ↔ Visualization ↔ Export)
Shapr3D segregates the modeling experience from the high-quality visualization, but makes it easy to go back and forth. The Mermaid diagram below illustrates a typical workflow in Shapr3D:
flowchart LR
    A[Modeling Space<br/>(Sketch & Extrude<br/>with basic shading)] -- Switch to Visualization --> B[Visualization Space<br/>(Apply Materials & HDRI<br/>with real-time render)];
    B -- Adjust camera/DOF --> B;
    B -- Export images/AR --> C[(Share/Export<br/>(PNG, USDZ for AR))];
    C -- Return to modeling --> A;
In this flow: The user does precise modeling in a fast, responsive viewport (A). When ready to present or evaluate appearance, they switch to Visualization (B) with one click
support.shapr3d.com
. In Visualization, they assign materials, pick environment lighting, adjust the camera angle or depth of field, and can capture screenshots or even view the model in AR (C). At any time they can go back to Modeling to make design changes (which update the Visualization view since it’s the same model data). This separation is great for performance (modeling mode stays light, while visualization can be heavier). OneCAD could adopt a similar approach: a lightweight modeling view (with optional edges, simple lighting) and a presentation mode (with full PBR, shadows, etc.). Alternatively, OneCAD might integrate some of these features directly in one view with toggles (Fusion 360 doesn’t have separate modes – it lets you turn shadows/AO on in the modeling viewport as needed). Shapr3D’s approach ensures a beginner user isn’t overwhelmed by visual settings during modeling – they only see those when they explicitly enter Visualization. OneCAD, aimed at makers, might simplify further by not even having separate modes but rather having easily accessible view settings (like a “Presentation” toggle). However, the mode separation is clean and could be implemented akin to how Shapr3D or SolidWorks do (SolidWorks has a Toggle RealView, Toggle Shadows, etc., all in the view menu – which in effect turns on/off these features on the fly).
Material Assignment Granularity
Shapr3D allows applying materials to entire bodies or individual faces
support.shapr3d.com
. This is important: often a part might have painted faces or different finishes on different faces. Shapr3D’s UX: select a face or body, then drag-drop material. If face, it only covers that face; if body, covers the whole body. They keep track of “Used Materials” per model for quick re-selection
support.shapr3d.com
. This granularity is something OneCAD should mimic – our data model via OCCT can assign different Aspect or material per face. We should ensure the rendering system can draw multiple materials on one mesh. Likely we’ll split by face groups or use a material ID buffer. Fusion 360 similarly allows face-level appearances. SolidWorks can apply “face colors” or “feature colors” that override the part’s material. So this is an expected CAD feature.
Export/Output
Shapr3D’s Visualization ultimately is about producing outputs: screenshot images or augmented reality (USDZ export)
support.shapr3d.com
. OneCAD should consider easy image export (with alpha or with a backdrop) at high resolution so users can showcase their designs. Also, possibly turntable animation or 3D web export down the line (Shapr3D has a Webviewer for sharing models
support.shapr3d.com
, which likely uses a simplified glTF and WebGL to show the model – a feature beyond rendering scope but related).
UX Lessons Summary
Have a distinct mode or easily accessible toggle for high-quality visualization so the user can intentionally enter “show me my model with full materials and lighting”.
Provide a material library with thumbnails for quick material application. No need for node editor for target users.
Support face-level and body-level material assignment with intuitive gestures (drag-drop or right-click “Apply to face/body”).
Make environment lighting choice simple (a few good HDRIs rather than infinite confusing options). Include a ground shadow option to ground the model.
Ensure the user can save views/cameras (Shapr3D allows saving up to 8 views
support.shapr3d.com
support.shapr3d.com
 – useful for returning to specific angles).
Provide export tools to get the visuals out (screenshots, AR, or exporting the model with materials to other software).
Overall, Shapr3D’s visualization pipeline confirms that focusing on IBL, quality shadows, good materials, and an easy workflow to toggle these on is the right strategy. OneCAD can aim to match that level of visual polish by version 1.5.
OneCAD Rendering Implementation Blueprint
Based on the requirements and the techniques discussed, here is a plan for implementing OneCAD’s rendering engine in a stepwise, integrable manner:
Leverage OCCT for Geometry and Basic Rendering Context
OneCAD uses OpenCASCADE (OCCT) as its geometry kernel. OCCT provides a viewer framework (V3d_View, AIS_InteractiveContext, etc.) which can render objects with OpenGL. However, to achieve modern PBR, we will need to bypass or extend parts of OCCT’s rendering. The strategy:
Tessellation via OCCT: Use OCCT’s BRepMesh or GPU tessellator to generate triangle meshes from the CAD Brep shapes. We will do this on demand and progressively (per the spec’s progressive tessellation) – coarse tessellation first (with large deflection, e.g. 1mm) followed by finer in background. OCCT can provide triangulations on faces, which we can collect into our mesh data structures. Each face can carry a material ID for later shading (especially if faces have different materials).
OCCT Scene Integration (AIS/V3d): We can use AIS_InteractiveObject subclasses or the Graphic3d layer to manage object transforms, selection highlighting, and possibly culling. OCCT’s viewer will handle camera projection, selection (e.g. projecting rays to pick shapes), and we can obtain the OpenGL context from Qt’s QOpenGLWidget that is shared with OCCT (the spec notes Qt RHI OpenGL backend is used for context sharing). We might disable OCCT’s default shading on objects or use a custom shader program via Graphic3d_ShaderManager to plug in our PBR shader if possible. Alternatively, we mark all AIS shapes to display in a neutral way (like white unlit) and then overlay our own rendering.
Rendering Backend (OpenGL 4.1 vs Metal): On macOS, we have the challenge that OpenGL is deprecated and limited to 4.1 (no compute shaders or modern debugging). However, Apple’s GL 4.1 implementation supports enough for PBR (we can do cubemaps, FBOs, etc.). Reverse-Z is tricky in GL4.1 since glClipControl is not available; but OCCT provides ReverseDepth via tricks – we should investigate if OCCT’s built-in Reverse-Z can be utilized (OCCT might internally set a projection matrix with reversed depth and adjust depth function, as their code snippet suggests enabling it). If not, we may restrict Reverse-Z to when using Metal/Vulkan (in the future). For now, assume we can do it with OCCT’s help in GL.
We will likely target OpenGL first (for portability to Intel Macs later too), using Qt’s QOpenGLWidget. Qt6’s Render Hardware Interface (RHI) could allow using Metal by writing backend-agnostic code, but that’s advanced; possibly we use GL on Mac for now, and plan a Metal migration later (since Apple Silicon has excellent Metal support, maybe via MoltenGL we stick to GL, but better to go Metal eventually for performance). This is a significant design decision: possibly use bgfx or another library that abstracts GL/Metal (discussed in Build vs Buy). For blueprint, assume OpenGL to start, because OCCT is known to work with it and Qt can host it easily.
PBR Shader Implementation
Implement a ubershader for OneCAD’s surfaces: a single GLSL shader (or Metal shader) that implements the Cook–Torrance BRDF with parameters: baseColor, metalness, roughness, plus optionally normal map and emissive. This shader will be used for all opaque objects. It will sample from the environment cubemap(s) for IBL. Key parts:
Uniforms: environment prefiltered map, irradiance map, BRDF LUT, perhaps light direction (for sun) and shadow map if shadows enabled.
Inputs per vertex: position, normal, tangent (for normal mapping), UV (for any textures).
Material params: could be per-object uniforms (e.g. a uniform vec3 baseColor, float metal, float rough) for uniform materials, or textures for those channels if assigned. Many CAD uses will be just solid colors (e.g. red plastic = baseColor red, metal=0, rough=0.4).
Shading pipeline: For each fragment, compute NdotL for environment diffuse (sample irradiance cube), NdotV etc. for specular (sample prefiltered env cube with reflection vector, MIP level based on roughness, then multiply by Fresnel and LUT term)
cdn2.unrealengine.com
. Sum them along with any direct light (if a sun light is used for shadows, add a Phong directional light term for it).
Gamma/Tone: Apply tone mapping curve or at least gamma at end.
Testing can be done with a test environment map to ensure reflections and lighting look right (we’ll need some HDRI in assets).
Environment Lighting & HDRI Pipeline
We will need to integrate IBL generation. We can either preprocess a set of HDRIs (ship with the app a few environment maps that are already processed into specular MIP chain and irradiance) or generate at runtime. For flexibility, perhaps precompute to save runtime cost. E.g. include 3–5 environments (studio, outdoor sunny, overcast, dark matte) as DDS or as .ktx cubemaps with mips. If user loads a custom HDR, we can on-the-fly compute its convolution (this can be done in a secondary thread if using compute shader or offline tool – maybe not initial focus). The engine will load the environment cubemap, create an irradiance cubemap (size ~32 or 64) by filtering, and a prefilter cubemap (size ~128 or 256 with mips). Also load the 2D BRDF LUT (we can precompute that 256x256 PNG using known formula). Many implementations include a default LUT (filament, UE4 have reference ones). Switching environment at runtime means just binding different cubemaps – straightforward.
Progressive Mesh Update & LOD Control
As per spec, when a modeling operation completes, we display the coarse mesh immediately. Implementation: after an operation, call OCCT to tessellate with coarse tolerance (1mm). That mesh is sent to GPU (VBO). Simultaneously, spawn a thread (or use OCCT’s built-in deferred meshing) to compute a finer mesh (0.1mm). When done, replace the mesh in the scene. Potentially do an even finer if needed. We will manage a mesh cache so that once fine is computed, it’s stored (so rotating the view doesn’t redo it). During camera navigation, as spec says, we drop to coarse LOD. We can implement this by simply swapping all meshes to a lower LOD version (or adjusting tessellation dynamically if memory allows storing multiple LODs). A simpler heuristic: just don’t show finer mesh until navigation stops (which is essentially what progressive does anyway). Also turning off MSAA and heavy effects during motion – we can detect camera moves via Qt events and temporarily toggle a low-quality mode in the renderer (could reduce sample count, etc.).
Selection Highlighting and Edges
We will likely still rely on OCCT’s AIS for selection (it can compute which face or edge is under cursor). But for rendering highlight: simplest is to render the selected object with a highlight color or outline. OCCT can also highlight via its context, but that might conflict with our custom shader. Perhaps we do selection highlighting by rendering the selected object(s) in a second pass with a color outline or an emission overlay. Edges: We implement edges by extracting the outer wireframe of the tessellation or using OCCT’s topological edges projected. Possibly easiest: take the triangulated model’s edges – but that includes all triangle edges (too many). Better: use the CAD edges classified by feature. We know which edges are silhouette candidates if angle between faces > threshold. We can either compute that each frame for silhouettes, or just draw all sharp edges and let the depth buffer hide the ones not visible (with slight offset). Polygon offset approach: Render filled geometry first. Then enable glPolygonOffset and render the geometry in wireframe mode (or render line primitives of the edges) in a constant color. By offsetting, the lines will appear on top
stackoverflow.com
. We must be careful to not offset too far (causing “disocclusion” where edges appear disconnected). A factor of 1, units 1 as in the example
stackoverflow.com
 is usually fine, but we might tweak. Another approach is drawing edges in a separate pass using the depth buffer with a slight bias on line depth. Either way, it’s straightforward. We will give the user an option to turn off edges (for a pure visual mode).
Reverse-Z and Depth Precision
We will attempt to enable Reverse-Z from day one, given large CAD scales. In OpenGL 4.1, we can simulate it by using a floating depth buffer, clearing depth to 0 and in the projection matrix flipping near/far (there are known techniques, though not as elegant as glClipControl). We saw OCCT has an internal flag – perhaps their GL engine already does the needed tweaks if we set that. If so, we just call that on our V3d_View and we get log depth precision. That would be ideal. We must also then use GL_GREATER depth test instead of less (since depth = 1 is at near plane in reverse). This is a detail but important. If reverse-Z doesn’t work out easily, we’ll set a relatively large far/near ratio and hope for the best in v1.0, then fix later. But since the spec explicitly calls it out, we will pursue it. With reverse-Z, we can keep near plane at, say, 0.1 or 1mm, and far at 1e6 mm (1 km), and still have enough precision.
Integration with Qt (on-demand rendering)
We will use QOpenGLWidget which gives us a paintGL() we can call via widget->update() when needed. We will not swap buffers continuously. Instead, connect signals: whenever the model changes or camera moves, we schedule an update. Qt will then call paintGL once. If the user is dragging, we might call update continuously (but Qt can merge those events to ~60fps). We ensure VSync is on (spec says VSync enabled). So the main loop is event-driven. On idle, no GPU usage. On camera inertia (if we implement inertia rotation as spec wants), we’d schedule continuous updates until inertia stops, then stop. That aligns with the spec’s mermaid diagram.
Material System and Data Structures
We’ll define a Material class with fields: baseColor (vec3), metalness, roughness, maybe an enum for type (or we just use those values to represent anything: e.g. Glass = metal=0, rough=0, with transmission flag – for now we can treat dielectric vs metal via metalness). Possibly an “isTransparent” flag and opacity for handling alpha. We’ll maintain a list of materials in the scene (like Shapr3D’s “Used Materials” list
support.shapr3d.com
) for easy reuse. Each face or object references a material. On the GPU, we can have a uniform array of material data or simply use multiple draw calls for different materials. Given CAD models may not have too many distinct materials, multiple draw calls is fine. Selection highlighting: We likely use a simple override – when an object is selected, we can either draw it again with a highlight color (additive blend or color mask). Or simpler: we temporarily change its material to a special highlight material (bright outline or color). OCCT usually does highlights by XOR drawing or overlay plane – but we might not use that technique. A common approach: draw the object in wireframe bright color on top (with polygon offset) – effectively an outline highlight. Alternatively, render a slightly scaled version in outline (makes a halo). We can experiment but it’s not the core focus here.
Putting it Together
In code architecture terms, we’ll have a Renderer module that manages:
Shaders (compiled at startup).
Environment maps (loaded from resources).
A Material library.
Mesh data (VAOs/VBOs for geometry).
Render passes (main opaque pass, edge overlay pass, maybe UI/2D overlays).
It interfaces with OCCT for camera parameters (get projection matrices from V3d_View) and perhaps for selection (we feed mouse position to OCCT’s selector to get an entity, then highlight via our engine).
We will ensure thread safety for background tessellation: if a fine mesh comes in, we might need to upload it to GPU in the next frame. We’ll use Qt’s signals or a thread-safe queue to inform the main thread to replace geometry. Finally, testing on Mac (Apple Silicon) – we will use Xcode/Metal debugging possibly if we try a Metal backend in future. But for v1, using GL through Qt means we can rely on the driver. Performance on M1 with a moderate model (100k triangles) with these effects (IBL, SSAO, shadow map at 2k, MSAA4x) should easily hit 30-60fps given Apple’s GPU power, as long as we optimize obvious things (don’t redraw if not needed, use frustum culling via BVH if scene large – OCCT provides BVH culling we can use).
Roadmap and Risk Management
Finally, we outline a phased roadmap for implementing these features in OneCAD, along with a risk register addressing potential challenges.
Development Roadmap
flowchart TB
    subgraph v1.0 Basic Visuals (6-9 months)
    v1a([OpenGL/Metal context<br>+ OCCT integration]) --> v1b([Basic Phong/Blinn shader<br>(flat shading, no PBR yet)]);
    v1b --> v1c([Geometry extraction<br>and progressive tessellation]);
    v1c --> v1d([Edge overlay rendering<br>(Shaded+Edges mode)✅]);
    v1d --> v1e([Selection highlighting<br>and hidden edge toggle]);
    v1e --> v1f([On-demand redraw loop<br>(events & inertia)✅]);
    end
    subgraph v1.5 Enhanced Realism (3-4 months)
    v15a([PBR shader: Cook-Torrance<br>with metal-rough params] );
    v15b([Material library UI<br>(preset materials)] );
    v15c([Environment IBL lighting<br>+ HDRI presets✅] );
    v15d([Shadow mapping (1 light)<br>+ ground plane shadows✳] );
    v15e([SSAO implementation✳] );
    v15f([Tone mapping (Filmic curve)✅] );
    v15a --> v15b --> v15c --> v15d --> v15e --> v15f;
    end
    subgraph v2.0 Polished & Advanced (6+ months)
    v2a([Refinements: normal maps,<br>transparency with OIT✳] );
    v2b([Depth of Field, camera control] );
    v2c([Optional path-trace mode<br>(Cycles/ProRender integration)✳] );
    v2d([Performance tuning,<br>Metal/Vulkan backend] );
    end
    v15f --> v2a;
    v2a --> v2b --> v2c --> v2d;
v1.0 – Basic Visuals: The focus is getting a working interactive viewport with essential CAD features. This includes hooking up the rendering context (OpenGL via Qt, with possible future Metal compatibility), extracting geometry from OCCT (initially maybe static tessellation without progressive refinement to start, then add the progressive thread). Shading can start non-PBR (even a simple headlight Lambertian shading) just to have something, but we will include edges from the get-go since that’s crucial for modeling. Reverse-Z can be tackled here by enabling OCCT’s flag, ensuring large models behave. By v1.0, the user should see a shaded model with edges and be able to orbit/pan/zoom with no lag (on-demand redraw working). Selection should highlight geometry (perhaps just color highlight). Materials might be basic (just per-body color). v1.5 – Enhanced Realism: This version upgrades the visual fidelity. We implement the full PBR shader and material system (leveraging the groundwork of v1.0 geometry pipeline). We introduce environment lighting – likely shipping a few environment maps and adding UI to choose them. Realistic shadows come in (single sun light casting shadow on model and ground). SSAO is added to deepen contacts. Tone mapping is applied so the shiny materials and bright environments look good (not clamped). The material UI now has a library – user can pick e.g. “Steel – Brushed” and that corresponds to base color gray, metal=1, rough=0.5 with a preset texture maybe. By v1.5, OneCAD’s viewport should look comparable to Shapr3D’s Visualization for static models, and still be interactive. We also ensure things like MSAA are on for quality. It’s called 1.5 because perhaps it’s not a fully separate release but an intermediate big update focused on visualization improvements. v2.0 – Polished & Advanced Features: In this stage, we tackle more advanced or niche features. Normal mapping and texture mapping support can be added so that materials can have bump details or stickers/decals. Proper transparency via OIT would be implemented so that if users model glass or need to see through, it works without glitches. Depth of Field and camera effects can be added to the camera controls (UI slider for focus distance and aperture). We also consider an optional path-traced render preview: this could be done by integrating an external renderer (e.g., call Blender’s Cycles via Python in background, or integrate AMD’s Radeon ProRender SDK which is GPU-accelerated and could run inside OneCAD to produce photoreal results
amd.com
). This feature would let users click “Ray Traced Render” and get a noise-converging high-quality image for their design – useful for final presentation beyond the real-time viewport. It’s a nice differentiator for a CAD aimed at makers who might want near-photographic images of their 3D printed part in context. However, it’s a significant effort, so it’s put in v2. Finally, by v2 we’d look at performance tuning and possibly moving to a more modern API (if we started on OpenGL, evaluate switching to Metal or Vulkan for longevity). Perhaps by v2, Qt will have improved RHI for Metal and we can migrate seamlessly, or use bgfx. Throughout these stages, backward steps may be needed for stability and performance issues identified by users. But this roadmap hits the key incremental improvements.
Risk Register
Implementing a custom CAD renderer comes with various risks. We list the major ones along with impact and mitigation:
Risk	Impact (H=High, M=Medium, L=Low)	Mitigation Strategy
Performance on Large Models – The combination of PBR, shadows, edges on models with millions of triangles might slow down interaction.	H (if viewport lags, users can’t model effectively)	Use dynamic LOD: during navigation, drop tessellation and effect quality. Implement frustum culling (OCCT BVH). Allow users to toggle effects (e.g. turn off shadows) for heavy scenes. Profile and optimize shader (avoid too many texture lookups). Possibly provide a “Performance mode” that simplifies shading if needed.
Apple Metal vs OpenGL Deprecation – Relying on OpenGL 4.1 on macOS could become problematic (Apple may remove it eventually). Metal implementation might be needed.	M (long term maintenance risk, not immediate crash)	Abstract the rendering layer such that switching backend is possible (consider using bgfx or MoltenVK). In short term, ensure our GLSL is compatible with Metal’s Shader Language via a tool or consider using Vulkan via MoltenVK which can target Metal. Monitor Qt’s RHI progress – Qt6 can use Metal through QSG, but for widgets we might wait or handle context ourselves. We plan for a backend switch in v2.x when stable.
Integration with OCCT – Possibly OCCT’s viewer might conflict with our custom rendering (e.g. OCCT wants to draw its default scene too).	M	Use OCCT in off-screen mode or minimal mode. For instance, we can subclass AIS_InteractiveObject to have OCCT only handle selection and not draw anything (by using a custom shader that discards fragments or by drawing into our own FBO). Another approach: intercept OCCT’s rendering by customizing its shaders to our own – OCCT 7.5+ allows GLSL shader override, so we could feed our PBR shader through that pipeline, preserving OCCT’s structure (this is advanced, but an option if we find too much conflict). Test early with simple OCCT shape to see if edges can be turned off and replaced by ours, etc.
Correctness of PBR – Ensuring our PBR looks “right” compared to standards (so materials appear as expected) is challenging, especially with tone mapping and different viewing conditions. If it looks wrong, users might think OneCAD’s visuals are poor.	L (visual quality risk, but not functional break)	Use well-known reference values (for instance, verify that our gold material with certain F0 looks like gold by comparing with glTF viewer). We will use the glTF spec and sample models as test – import a glTF model with known good PBR and see if OneCAD matches it. Also, keep option for users to adjust exposure or turn off tone mapping if needed. Collect feedback in beta testing on material appearance and refine BRDF or tone curves accordingly.
Development Complexity & Time – Building a custom engine with these features is non-trivial. There’s a risk of slipping schedule or bugs.	M/H (could delay releases)	Prioritize features by importance: e.g. edges and basic shading are must-have (so deliver those first, as in v1.0), whereas SSR, SSGI are optional (we decided to skip). Consider adopting existing components: e.g. if time is short, integrate an open-source PBR renderer (Filament or three.js via WebGL in a widget) as a stopgap – though this has its own challenges. Mitigation primarily via careful scope management and possibly a “build vs buy” evaluation (see next section) to offload some work.
Memory Footprint – Environment maps, multiple LOD meshes, etc., consume memory. On 8GB machines, this might be an issue if not managed.	L	Limit default environment resolution (e.g. 1K cube maps). Unload fine LOD if not needed (if user never zooms extremely, we can keep only medium LOD). Provide settings to purge or limit tessellation quality. Use compressed textures for environment (ASTC/BC6 if possible). Overall, CAD models are usually moderate size, but keep an eye on it.
User Preference Diversity – Some users may prefer a simple flat shading (for engineering clarity), others want full realism. One size might not fit all.	M	Provide easy toggles or modes (like Shaded vs Visualized in Shapr3D
support.shapr3d.com
). Ensure documentation explains how to switch. Possibly remember last mode. By catering to both (maybe a quick “presentation mode” button), we reduce user friction.
Transparency Sorting Issues – Without full OIT until v2, users might see incorrect layering of transparent objects in v1.x.	L	For early versions, document that transparency is basic. If it becomes a pain point, prioritize OIT earlier or restrict usage (e.g. allow only one object to be transparent at a time via UI, which we can handle by drawing it last). Mitigate by at least implementing back-to-front sort for separate objects as an interim solution.
Many of these risks are manageable with the chosen phased approach. The most critical for functionality is performance – which we address by adhering to on-demand rendering and quality downgrades during interaction per spec (a proven approach in CAD). The most critical for adoption is visual correctness and usability, which we address by following known conventions (glTF, Disney shader, etc.) and providing user-friendly toggles akin to Shapr3D/Blender presets.
Build vs Buy Analysis
A key architectural decision is whether to implement our PBR rendering in-house or integrate an existing rendering engine/library. We consider some options:
Custom Implementation (Build): Writing our own rendering engine gives full control and tight integration with OCCT and Qt. We can tailor it precisely to CAD needs (e.g. edge rendering, CAD units, etc.). As outlined, it’s feasible with modern OpenGL/Metal and relatively few dependencies. The downside is development effort and maintaining a rendering engine (bugs to fix, new GPU quirks, etc.). However, since our needs are specific and we’re not aiming for an entire game engine’s complexity, a custom approach is reasonable.
Use an Existing Real-time PBR Engine (Buy/integrate): Candidates include Google’s Filament (Apache 2.0 license) – a lightweight PBR renderer that supports glTF materials, IBL, shadows, etc., and has an API for adding objects. Filament could theoretically render our OCCT meshes with minimal shader writing. Similarly, bgfx is a rendering library (BSD license) that abstracts graphics APIs and includes some example shaders but not a full material system – we’d still have to implement PBR on top of it. Three.js (for Web, not directly applicable for a Qt desktop app without using WebGL context) is another reference for features but not directly integrable. Unreal Engine or Unity are too heavyweight and not license-compatible for embedding in an open-source CAD easily.
Use a GPU path tracer for final renders: AMD’s Radeon ProRender is a free (MIT license) library that provides photorealistic rendering with an API. We could use it for the “preview/render” mode (not for interactive dragging, but for user-invoked renders). It supports CAD formats and complex materials. The integration cost here is less about real-time and more about exporting our scene to it and displaying the result, which is an isolated case.
Tradeoffs:
Integration with Qt/OCCT: Our custom build can tie directly into QOpenGLWidget and use OCCT’s data. Filament, on the other hand, has its own driver and material definitions. It can run on Metal (it supports Metal via MoltenVK or directly I think) and OpenGL. If we tried Filament, we’d have to convert OCCT geometry to Filament meshes, materials to Filament’s Material instances, etc. It might bypass some OCCT selection or require reimplementing selection picking by ourselves (since Filament wouldn’t know OCCT’s shapes). Filament does bring advanced things like tempered reflections, etc., but we may not need all that.
Time to Market: Adopting an engine like Filament might save shader development time (their materials are tuned and validated). But it introduces learning curve and possibly constraints (for instance, Filament doesn’t natively do CAD-style edge outlines – we’d have to post-process anyway). If something doesn’t work, fixing it might be harder than in our own code. Bgfx is lower-level (it just handles device abstraction and some utilities), so using it means we still write the material system – its benefit is multi-API support. Multi-API is a concern because of Metal – however, Apple’s OpenGL is still functional now, and if that becomes an issue, we can plan to move to Metal via MoltenVK or bgfx later.
Licensing: Our project is open-source (MIT/Apache per spec). Filament is Apache 2.0 – compatible. Bgfx is BSD – compatible. So no blocker. Proprietary engines (Unity, etc.) are not an option. Radeon ProRender is MIT – fine.
Quality and Support: Filament is production quality (used in Android apps, etc.), so using it could instantly give us a high-quality baseline (with tone mapping, MSAA, etc.). If we roll our own, we have to calibrate everything. However, our needs are narrower – we’re not doing skinning, particle effects, etc. Filament might be an overkill and could add overhead (it’s optimized for mobile too, but it’s still a general engine – e.g. its lighting model includes more stuff like subsurface, clearcoat; we might not expose those anyway).
Conclusion: On balance, the hybrid approach seems best: Build the core features ourselves, to ensure integration with OCCT and CAD-specific needs (edges, selection, etc.), but keep the door open to use external renderers for specific tasks (like ProRender for final quality renders). The custom route ensures we meet the spec’s on-demand and progressive requirements tightly. For instance, hooking progressive mesh LOD into Filament could be as much work as doing it from scratch. That said, we will reuse known algorithms and maybe even shader code from open sources. For example, we could lift a reference GGX shader from learnopengl or Filament’s documentation to avoid writing from zero (Filament’s material model is documented and similar to what we need). Also, if down the line, maintenance becomes burdensome, we could consider swapping out parts (for example, using bgfx to handle the GL vs Metal difference, while keeping our shader logic – bgfx would manage the rendering backend so we don’t have to write separate Metal code; this could mitigate the Apple GL deprecation risk). The bgfx documentation highlight it’s a “Bring Your Own Engine” style library
bkaradzic.github.io
, meaning it won’t impose a scene graph or materials, which suits our need to inject CAD-specific things. To put simply: We Build now because it’s straightforward for initial requirements and aligns with how many CAD tools evolve (most have custom in-house viewports tuned for CAD). We Buy (or borrow) later for specialized tasks: e.g., incorporate Radeon ProRender for a one-click photoreal render (it’s essentially an offline renderer we don’t have to write), and possibly incorporate bgfx or Vulkan to get Metal support if OpenGL truly sunsets. By keeping our code modular, these additions can be done without a full rewrite. In summary, OneCAD’s rendering engine will be custom-built for real-time interaction, ensuring we meet all specified requirements (on-demand loop, progressive refinement, etc.), and we will integrate external solutions where they provide clear value (like a final raytracer for marketing-quality images, or a rendering backend abstraction to future-proof API support). This approach balances development control with leveraging existing technology appropriately, de-risking the project in the long run.
Appendix: Bibliography
OneCAD Software Specification – Rendering System. OneCAD Internal SPECIFICATION.md, Version 1.4, Section 18, details on rendering config and display modes (OpenGL on-demand, MSAA, Reverse-Z, etc.). Accessed on 2026-01-03.
OneCAD Software Specification – Progressive Tessellation. OneCAD Internal SPECIFICATION.md, Version 1.4, Section 18.4–18.5, outlines multi-stage tessellation tolerances and dynamic quality adjustment during navigation. Accessed on 2026-01-03.
OneCAD Software Specification – Reverse Depth Buffer. OneCAD Internal SPECIFICATION.md, Version 1.4, Section 18.6, note on enabling reverse depth in OCCT (ToReverseDepth flag) to improve Z precision for large models. Accessed on 2026-01-03.
Shapr3D Help Center – Views and Appearance. Explains Shapr3D’s viewport settings: projection slider, Visualized shading mode for enhanced edges, etc. Shapr3D Support Article, updated Sept 23, 2025. Accessed on 2026-01-03. 
support.shapr3d.com
support.shapr3d.com
Shapr3D Help Center – Visualization. Describes Shapr3D’s Visualization mode with real-time rendering: material library (~100 materials), environment/HDRI options, camera and DOF settings, applying materials to bodies or faces. Shapr3D Support Article, updated Nov 06, 2025. Accessed on 2026-01-03. 
support.shapr3d.com
support.shapr3d.com
Unity HDRP Documentation – Contact Shadows. Unity Manual (HDRP), describes screen-space ray-marched contact shadows that add small-scale shadow detail in close contact areas. Accessed on 2026-01-03. 
docs.unity3d.com
threejs.org
Panos Karabelas – Screen Space Shadows (Contact Shadows). Personal Blog post, Jul 2020. Explains implementing screen-space (contact) shadows, notes they only appear when occluder is very close (hence “almost make contact”). Accessed on 2026-01-03. 
panoskarabelas.com
panoskarabelas.com
3D Game Shaders For Beginners – Screen Space Reflections. Tutorial explaining SSR: reflecting the screen onto itself via ray marching, and its limitations (fails for off-screen geometry). Accessed on 2026-01-03. 
lettier.github.io
Khronos glTF 2.0 Specification – PBR Materials. Official spec describing the metallic-roughness material model (baseColorFactor/Texture, metallicFactor, roughnessFactor, texture channels). The glTF model ensures consistent PBR material definitions across engines. Accessed on 2026-01-03. 
registry.khronos.org
registry.khronos.org
Filament Documentation – Standard PBR Model. Romain Guy, Mathias Agopian, “Physically Based Rendering in Filament”, Google, 2018. Summarizes Filament’s standard model: Cook-Torrance microfacet with GGX NDF, Smith GGX geometry, Schlick Fresnel, Lambertian diffuse
google.github.io
. Confirms industry adoption of these components. Accessed on 2026-01-03.
OpenGL Polygon Offset for Hidden Line Removal. StackOverflow Q&A “Draw wireframe mesh with hidden lines removed” (2015) by user wcochran. Describes the technique of drawing polygon edges, then filled polys with polygon offset to avoid z-fighting, illustrating with wireframe images. Accessed on 2026-01-03. 
stackoverflow.com
SolidWorks RealView and Ambient Occlusion (AMD Guide). AMD Radeon Pro Guide: SolidWorks 2023 Visualization, 2022. Discusses SolidWorks’ real-time viewport features: RealView graphics, shadows, ambient occlusion, order-independent transparency. Mentions that RealView + AO give fully interactive views, though not full global lighting
amd.com
. Also describes OIT improving transparent blending
amd.com
. Accessed on 2026-01-03. 
amd.com
amd.com
SolidWorks Help – Ambient Occlusion. SolidWorks 2026 Documentation (web). Defines ambient occlusion as a global lighting method that attenuates ambient light in occluded areas, adding realism
help.solidworks.com
. (Full content not shown due to web help format). Accessed on 2026-01-03.
Disney principled BRDF. Brent Burley, “Physically-Based Shading at Disney”, Disney Animation, 2012. (Not directly cited above, but foundational reference for Principled BSDF, explaining the rationale of an artist-friendly unified shader).
Unreal Engine 4 – Real Shading Model (Brian Karis, Epic Games, 2013). (Siggraph course notes). Describes the split-sum integration for IBL, use of prefiltered environment maps and a 2D BRDF LUT
cdn2.unrealengine.com
. Demonstrates importance of using a Fresnel approximation (Schlick) and pre-convolved environment for performance
cdn2.unrealengine.com
. Accessed on 2026-01-03.
GitHub – bgfx Overview. bgfx project README, states it’s a cross-platform, API-agnostic rendering library (Direct3D 9/11/12, OpenGL, Metal, Vulkan) under BSD license, “bring your own engine” style
bkaradzic.github.io
. Useful to abstract low-level API differences. Accessed on 2026-01-03.
Blender Manual – Principled BSDF. Blender 2.80 Manual, notes that Principled BSDF combines multiple layers into one easy node, based on Disney “PBR” shader
docs.blender.org
. Shows that even advanced 3D software use a unified node for PBR, supporting our approach. Accessed on 2026-01-03.
Shapr3D Community Forum – User Feedback on Visualization. Various threads (e.g., “Future features: need basic lighting, AO, cubemap…”, 2022) where users discuss desired visualization features like ambient occlusion, shadow direction control
discourse.shapr3d.com
. Indicates user expectations and helps prioritize implementing AO and lighting controls in OneCAD’s roadmap. Accessed on 2026-01-03.
Citácie
SPECIFICATION.md

file://file_00000000443871f4a90157fe618b0ce0
SPECIFICATION.md

file://file_00000000443871f4a90157fe618b0ce0
SPECIFICATION.md

file://file_00000000443871f4a90157fe618b0ce0
SPECIFICATION.md

file://file_00000000443871f4a90157fe618b0ce0
SPECIFICATION.md

file://file_00000000443871f4a90157fe618b0ce0
SPECIFICATION.md

file://file_00000000443871f4a90157fe618b0ce0
SPECIFICATION.md

file://file_00000000443871f4a90157fe618b0ce0
SPECIFICATION.md

file://file_00000000443871f4a90157fe618b0ce0
SPECIFICATION.md

file://file_00000000443871f4a90157fe618b0ce0
SPECIFICATION.md

file://file_00000000443871f4a90157fe618b0ce0
SPECIFICATION.md

file://file_00000000443871f4a90157fe618b0ce0
SPECIFICATION.md

file://file_00000000443871f4a90157fe618b0ce0
SPECIFICATION.md

file://file_00000000443871f4a90157fe618b0ce0
SPECIFICATION.md

file://file_00000000443871f4a90157fe618b0ce0

google.github.io

https://google.github.io/filament/Filament.md.html

google.github.io

https://google.github.io/filament/Filament.md.html

google.github.io

https://google.github.io/filament/Filament.md.html

google.github.io

https://google.github.io/filament/Filament.md.html

google.github.io

https://google.github.io/filament/Filament.md.html

google.github.io

https://google.github.io/filament/Filament.md.html

google.github.io

https://google.github.io/filament/Filament.md.html

glTF™ 2.0 Specification

https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html

glTF™ 2.0 Specification

https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html

glTF™ 2.0 Specification

https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html

glTF™ 2.0 Specification

https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html
https://cdn2.unrealengine.com/Resources/files/2013SiggraphPresentationsNotes-26915738.pdf
https://cdn2.unrealengine.com/Resources/files/2013SiggraphPresentationsNotes-26915738.pdf
https://cdn2.unrealengine.com/Resources/files/2013SiggraphPresentationsNotes-26915738.pdf

Visualization – Shapr3D Help Center

https://support.shapr3d.com/hc/en-us/articles/7874459464092-Visualization
https://cdn2.unrealengine.com/Resources/files/2013SiggraphPresentationsNotes-26915738.pdf

GitHub - mmikk/MikkTSpace: A common standard for tangent space used in baking tools to produce normal maps.

https://github.com/mmikk/MikkTSpace

Color Grading and the Filmic Tonemapper in Unreal Engine

https://dev.epicgames.com/documentation/en-us/unreal-engine/color-grading-and-the-filmic-tonemapper-in-unreal-engine

What is Tone Mapping / ACES (ACEScg)?

https://www.danthree.studio/en/glossary/tone-mapping-aces-acescg

Contact Shadows | High Definition RP | 14.0.12 - Unity - Manual

https://docs.unity3d.com/Packages/com.unity.render-pipelines.high-definition@14.0/manual/Override-Contact-Shadows.html

SSSNode – three.js docs

https://threejs.org/docs/pages/SSSNode.html

Screen space shadows

https://panoskarabelas.com/posts/screen_space_shadows/

Screen space shadows

https://panoskarabelas.com/posts/screen_space_shadows/

Assembly Performance Best Practices - Hawk Ridge Systems

https://support.hawkridgesys.com/hc/en-us/articles/360006864531-Assembly-Performance-Best-Practices

Ambient Occlusion - 2026 - SOLIDWORKS Design Help

https://help.solidworks.com/2026/english/SWConnected/swdotworks/t_ambient_occlusion.htm

Add more realism to your SolidWorks view with Ambient Occlusion

https://www.javelin-tech.com/blog/2014/05/add-realism-ambient-occlusion/
Screen Space Reflection | 3D Game Shaders For Beginners

https://lettier.github.io/3d-game-shaders-for-beginners/screen-space-reflection.html

Screen Space Reflections (SSR) Rendering Pipeline

https://doc.babylonjs.com/features/featuresDeepDive/postProcesses/SSRRenderingPipeline

what is Screen space Global illumination (SSGI). When to use it ...

https://forums.unrealengine.com/t/what-is-screen-space-global-illumination-ssgi-when-to-use-it-how-it-differs-from-other-gi/1174099
Casual Effects: Weighted, Blended Order-Independent Transparency

http://casual-effects.blogspot.com/2014/03/weighted-blended-order-independent.html
Casual Effects: Weighted, Blended Order-Independent Transparency

http://casual-effects.blogspot.com/2014/03/weighted-blended-order-independent.html
https://www.amd.com/content/dam/amd/en/documents/products/graphics/workstation/solidworks-2023-guide.pdf
Casual Effects: Weighted, Blended Order-Independent Transparency

http://casual-effects.blogspot.com/2014/03/weighted-blended-order-independent.html

Views and Appearance – Shapr3D Help Center

https://support.shapr3d.com/hc/en-us/articles/7873944390812-Views-and-Appearance

android - How do I draw a wireframe mesh with hidden lines removed in OpenGL ES? - Stack Overflow

https://stackoverflow.com/questions/30567044/how-do-i-draw-a-wireframe-mesh-with-hidden-lines-removed-in-opengl-es

Views and Appearance – Shapr3D Help Center

https://support.shapr3d.com/hc/en-us/articles/7873944390812-Views-and-Appearance

Views and Appearance – Shapr3D Help Center

https://support.shapr3d.com/hc/en-us/articles/7873944390812-Views-and-Appearance

Views and Appearance – Shapr3D Help Center

https://support.shapr3d.com/hc/en-us/articles/7873944390812-Views-and-Appearance

Principled BSDF — Blender Manual - Shader

https://docs.blender.org/manual/en/2.80/render/shader_nodes/shader/principled.html

Visualization – Shapr3D Help Center

https://support.shapr3d.com/hc/en-us/articles/7874459464092-Visualization

Visualization – Shapr3D Help Center

https://support.shapr3d.com/hc/en-us/articles/7874459464092-Visualization

Visualization – Shapr3D Help Center

https://support.shapr3d.com/hc/en-us/articles/7874459464092-Visualization

Visualization – Shapr3D Help Center

https://support.shapr3d.com/hc/en-us/articles/7874459464092-Visualization

Visualization – Shapr3D Help Center

https://support.shapr3d.com/hc/en-us/articles/7874459464092-Visualization

Removing Shadows in Visualization - Shapr3D Community

https://discourse.shapr3d.com/t/removing-shadows-in-visualization/24532

Visualization – Shapr3D Help Center

https://support.shapr3d.com/hc/en-us/articles/7874459464092-Visualization

Future features + fixes? - Need help? We are here.

https://discourse.shapr3d.com/t/future-features-fixes/9225

Views and Appearance – Shapr3D Help Center

https://support.shapr3d.com/hc/en-us/articles/7873944390812-Views-and-Appearance
SPECIFICATION.md

file://file_00000000443871f4a90157fe618b0ce0

Visualization – Shapr3D Help Center

https://support.shapr3d.com/hc/en-us/articles/7874459464092-Visualization
https://www.amd.com/content/dam/amd/en/documents/products/graphics/workstation/solidworks-2023-guide.pdf

Visualization – Shapr3D Help Center

https://support.shapr3d.com/hc/en-us/articles/7874459464092-Visualization

Visualization – Shapr3D Help Center

https://support.shapr3d.com/hc/en-us/articles/7874459464092-Visualization

Visualization – Shapr3D Help Center

https://support.shapr3d.com/hc/en-us/articles/7874459464092-Visualization

Visualization – Shapr3D Help Center

https://support.shapr3d.com/hc/en-us/articles/7874459464092-Visualization

Visualization – Shapr3D Help Center

https://support.shapr3d.com/hc/en-us/articles/7874459464092-Visualization

Views and Appearance – Shapr3D Help Center

https://support.shapr3d.com/hc/en-us/articles/7873944390812-Views-and-Appearance

Views and Appearance – Shapr3D Help Center

https://support.shapr3d.com/hc/en-us/articles/7873944390812-Views-and-Appearance
SPECIFICATION.md

file://file_00000000443871f4a90157fe618b0ce0

android - How do I draw a wireframe mesh with hidden lines removed in OpenGL ES? - Stack Overflow

https://stackoverflow.com/questions/30567044/how-do-i-draw-a-wireframe-mesh-with-hidden-lines-removed-in-opengl-es
SPECIFICATION.md

file://file_00000000443871f4a90157fe618b0ce0
CAM.md

file://file_0000000075bc71f489d2d7ea86cb4564
CAM.md

file://file_0000000075bc71f489d2d7ea86cb4564
CAM.md

file://file_0000000075bc71f489d2d7ea86cb4564
https://www.amd.com/content/dam/amd/en/documents/products/graphics/workstation/solidworks-2023-guide.pdf
SPECIFICATION.md

file://file_00000000443871f4a90157fe618b0ce0

Overview — bgfx 1.135.9039 documentation

https://bkaradzic.github.io/bgfx/overview.html
https://www.amd.com/content/dam/amd/en/documents/products/graphics/workstation/solidworks-2023-guide.pdf