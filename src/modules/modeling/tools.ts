/*
 * Modeling tool descriptors — the SINGLE source of truth for the tool palette.
 *
 * Previously this arrangement lived in `features/toolbar/toolbarConfig.ts` as two
 * static arrays. It now lives with the module that owns the tools; the toolbar
 * derives its arrays from here and the platform tool registry is fed from the
 * same table, so the two can never disagree.
 *
 * Separators are NOT entries: they are group boundaries. A group is a contiguous
 * run of priorities, and the toolbar draws a separator wherever the group changes
 * (ADR-0007 — `group` is consumer metadata, `priority` is the sort key).
 *
 * Priorities are spaced by 10 inside a group and by 100 between groups, leaving
 * room for a later tool to land in the right place without renumbering.
 */
import type { IconName } from "@/icons/paths";
import type { ModelTool, SketchTool } from "@/stores/toolStore";

export type ToolScope = "model" | "sketch";

interface ToolPresentation {
  readonly icon: IconName;
  readonly label: string;
  /**
   * The glyph shown in the toolbar. Deliberately separate from the key binding:
   * Measure binds ⇧/ but reads as "?", so the two are not mechanically derivable
   * from each other.
   */
  readonly shortcut: string;
  readonly group: string;
  readonly priority: number;
  /**
   * The flyout family this tool belongs to (opaque key). Members sharing a key
   * render as ONE split button: the lowest-priority member is the default, the
   * others live in its dropdown. Absent ⇒ the tool owns its own button.
   */
  readonly family?: string;
  /**
   * Absent from the floating toolbar's visible row — still registered (so
   * `⌘K`/its shortcut keep working, `FloatingToolbar.tsx`'s own
   * `hiddenToolGroups` precedent for "registered but not toolbar real estate"
   * already establishes this is a legitimate split), just presented somewhere
   * else instead. Set alongside {@link ToolPresentation.generatorsMenu} today;
   * a future non-generator use is free to set this alone.
   */
  readonly toolbarHidden?: boolean;
  /** Presented as a row in the title-bar "Generators" menu (`GeneratorsMenu.tsx`)
   *  instead of, or in addition to, the floating toolbar. */
  readonly generatorsMenu?: boolean;
}

/**
 * Discriminated on `scope`, so a sketch-only tool placed in the model table is a
 * COMPILE error rather than a runtime surprise — and so consumers narrow to the
 * right union without a cast.
 */
export type ModelingToolDescriptor =
  | (ToolPresentation & { readonly scope: "model"; readonly tool: ModelTool })
  | (ToolPresentation & { readonly scope: "sketch"; readonly tool: SketchTool });

export type ModelToolDescriptor = Extract<ModelingToolDescriptor, { scope: "model" }>;
export type SketchToolDescriptor = Extract<ModelingToolDescriptor, { scope: "sketch" }>;

export const MODEL_TOOL_DESCRIPTORS: readonly ModelToolDescriptor[] = [
  // Authoring surfaces: both of these produce something to draw ON, which is why
  // Datum sits here rather than with the solid ops below.
  { tool: "select", scope: "model", icon: "select", label: "Select", shortcut: "V", group: "model.create", priority: 100 },
  { tool: "sketch", scope: "model", icon: "sketch", label: "New sketch", shortcut: "S", group: "model.create", priority: 110 },
  { tool: "datum", scope: "model", icon: "datum", label: "Datum plane", shortcut: "D", group: "model.create", priority: 120 },

  // The solid-producing operations.
  { tool: "extrude", scope: "model", icon: "extrude", label: "Extrude", shortcut: "E", group: "model.solid", priority: 200 },
  { tool: "revolve", scope: "model", icon: "revolve", label: "Revolve", shortcut: "R", group: "model.solid", priority: 210 },
  { tool: "fillet", scope: "model", icon: "fillet", label: "Fillet / Chamfer", shortcut: "F", group: "model.solid", priority: 220 },
  { tool: "boolean", scope: "model", icon: "boolean", label: "Combine", shortcut: "B", group: "model.solid", priority: 230 },
  // A gear MINTS a body, but it is a GENERATED-SHAPE tool, not a general solid
  // op — it lives in the title-bar "Generators" menu (`GeneratorsMenu.tsx`),
  // not the floating toolbar (`toolbarHidden`), and registers itself there via
  // `generatorsMenu`. Own glyph (`icons/authored.ts gear`) — was
  // `circularPattern` (borrowed for its radial-repetition reading) until then.
  // `group`/`priority` are kept (still needed for the registry/ids machinery
  // and for a future toolbar re-enable) even though they no longer place a
  // visible button.
  { tool: "gear", scope: "model", icon: "gear", label: "Gear", shortcut: "⇧G", group: "model.solid", priority: 240, toolbarHidden: true, generatorsMenu: true },

  // Body-level modifiers. `pushpull` is the live registry glyph for "move this
  // face"; no new icon was minted for OffsetFace.
  { tool: "shell", scope: "model", icon: "shell", label: "Shell", shortcut: "K", group: "model.modify", priority: 300 },
  { tool: "offsetFace", scope: "model", icon: "pushpull", label: "Offset face", shortcut: "⇧O", group: "model.modify", priority: 310 },
  { tool: "hole", scope: "model", icon: "hole", label: "Hole", shortcut: "⇧H", group: "model.modify", priority: 320 },
  { tool: "linearPattern", scope: "model", icon: "linearPattern", label: "Linear pattern", shortcut: "P", group: "model.modify", priority: 330, family: "onecad.modeling.flyout.model.pattern" },
  // Circular repeats the same verb with the other arrangement, so the pair shares
  // one split button — the menu is where the two gestures differ.
  { tool: "circularPattern", scope: "model", icon: "circularPattern", label: "Circular pattern", shortcut: "C", group: "model.modify", priority: 340, family: "onecad.modeling.flyout.model.pattern" },
  { tool: "mirror", scope: "model", icon: "mirrorBody", label: "Mirror", shortcut: "M", group: "model.modify", priority: 350 },
  // A PLACEMENT, not a modelling op: it changes where a body is, never its shape.
  { tool: "transform", scope: "model", icon: "move", label: "Move", shortcut: "T", group: "model.modify", priority: 360 },

  // Alone at the tail: the only READ-ONLY tool in the set — it writes nothing to
  // the timeline, so it does not belong among the operations.
  { tool: "measure", scope: "model", icon: "measure", label: "Measure", shortcut: "?", group: "model.inspect", priority: 400 },
];

export const SKETCH_TOOL_DESCRIPTORS: readonly SketchToolDescriptor[] = [
  { tool: "select", scope: "sketch", icon: "select", label: "Select", shortcut: "V", group: "sketch.select", priority: 100 },

  { tool: "line", scope: "sketch", icon: "line", label: "Line", shortcut: "L", group: "sketch.draw", priority: 200 },
  { tool: "rect", scope: "sketch", icon: "rect", label: "Rectangle", shortcut: "R", group: "sketch.draw", priority: 210, family: "onecad.modeling.flyout.sketch.rect" },
  { tool: "centerRect", scope: "sketch", icon: "centerRect", label: "Center rectangle", shortcut: "⇧R", group: "sketch.draw", priority: 220, family: "onecad.modeling.flyout.sketch.rect" },
  { tool: "circle", scope: "sketch", icon: "circle", label: "Circle", shortcut: "C", group: "sketch.draw", priority: 230, family: "onecad.modeling.flyout.sketch.curve" },
  { tool: "ellipse", scope: "sketch", icon: "ellipse", label: "Ellipse", shortcut: "O", group: "sketch.draw", priority: 240, family: "onecad.modeling.flyout.sketch.curve" },
  { tool: "arc", scope: "sketch", icon: "arc", label: "Arc", shortcut: "A", group: "sketch.draw", priority: 250, family: "onecad.modeling.flyout.sketch.arc" },
  // The second arc gesture, next to the first: same shape, the other three inputs.
  { tool: "arc3p", scope: "sketch", icon: "arc3p", label: "3-point arc", shortcut: "⇧A", group: "sketch.draw", priority: 260, family: "onecad.modeling.flyout.sketch.arc" },

  { tool: "polygon", scope: "sketch", icon: "polygon", label: "Polygon", shortcut: "G", group: "sketch.shapes", priority: 300 },
  { tool: "slot", scope: "sketch", icon: "slot", label: "Slot", shortcut: "S", group: "sketch.shapes", priority: 310 },
  { tool: "point", scope: "sketch", icon: "point", label: "Point", shortcut: "P", group: "sketch.shapes", priority: 320 },

  { tool: "dimension", scope: "sketch", icon: "dimension", label: "Dimension", shortcut: "D", group: "sketch.edit", priority: 400 },
  { tool: "trim", scope: "sketch", icon: "trim", label: "Trim", shortcut: "T", group: "sketch.edit", priority: 410 },
  // Trim's counterpart, immediately after it: same pick gesture, opposite effect,
  // so the pair reads as one decision (cut back / grow out).
  { tool: "extend", scope: "sketch", icon: "extend", label: "Extend", shortcut: "⇧T", group: "sketch.edit", priority: 420 },
  { tool: "mirror", scope: "sketch", icon: "mirror", label: "Mirror", shortcut: "M", group: "sketch.edit", priority: 430 },
  // The 2D fillet reuses the model `fillet` glyph deliberately: it is the same
  // operation one dimension down, and the two live in different toolbars.
  { tool: "sketchFillet", scope: "sketch", icon: "fillet", label: "Fillet", shortcut: "F", group: "sketch.edit", priority: 440 },
  { tool: "sketchOffset", scope: "sketch", icon: "offset", label: "Offset", shortcut: "⇧O", group: "sketch.edit", priority: 450 },
];

export const MODELING_TOOL_DESCRIPTORS: readonly ModelingToolDescriptor[] = [
  ...MODEL_TOOL_DESCRIPTORS,
  ...SKETCH_TOOL_DESCRIPTORS,
];

export function descriptorsForScope(scope: ToolScope): readonly ModelingToolDescriptor[] {
  return scope === "sketch" ? SKETCH_TOOL_DESCRIPTORS : MODEL_TOOL_DESCRIPTORS;
}

export const modelTools = (): readonly ModelTool[] => MODEL_TOOL_DESCRIPTORS.map((d) => d.tool);

export const sketchTools = (): readonly SketchTool[] => SKETCH_TOOL_DESCRIPTORS.map((d) => d.tool);
