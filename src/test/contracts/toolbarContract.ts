/*
 * FROZEN toolbar contract — a literal copy of the floating toolbar as shipped
 * before the Platform refactor (order, grouping separators, icons, labels,
 * shortcut glyphs). See ./README.md before editing.
 */
import type { ToolEntry } from "@/features/toolbar/toolbarConfig";

export const MODEL_TOOLS_CONTRACT: readonly ToolEntry[] = [
  { id: "select", icon: "select", label: "Select", shortcut: "V" },
  { id: "sketch", icon: "sketch", label: "New sketch", shortcut: "S" },
  { id: "datum", icon: "datum", label: "Datum plane", shortcut: "D" },
  { sep: true },
  { id: "extrude", icon: "extrude", label: "Extrude", shortcut: "E" },
  { id: "revolve", icon: "revolve", label: "Revolve", shortcut: "R" },
  { id: "fillet", icon: "fillet", label: "Fillet / Chamfer", shortcut: "F" },
  { id: "boolean", icon: "boolean", label: "Combine", shortcut: "B" },
  { sep: true },
  { id: "shell", icon: "shell", label: "Shell", shortcut: "K" },
  { id: "offsetFace", icon: "pushpull", label: "Offset face", shortcut: "⇧O" },
  { id: "hole", icon: "hole", label: "Hole", shortcut: "⇧H" },
  { id: "linearPattern", icon: "linearPattern", label: "Linear pattern", shortcut: "P" },
  { id: "circularPattern", icon: "circularPattern", label: "Circular pattern", shortcut: "C" },
  { id: "mirror", icon: "mirrorBody", label: "Mirror", shortcut: "M" },
  { id: "transform", icon: "move", label: "Move", shortcut: "T" },
  { sep: true },
  { id: "measure", icon: "measure", label: "Measure", shortcut: "?" },
];

export const SKETCH_TOOLS_CONTRACT: readonly ToolEntry[] = [
  { id: "select", icon: "select", label: "Select", shortcut: "V" },
  { sep: true },
  { id: "line", icon: "line", label: "Line", shortcut: "L" },
  { id: "rect", icon: "rect", label: "Rectangle", shortcut: "R" },
  { id: "centerRect", icon: "centerRect", label: "Center rectangle", shortcut: "⇧R" },
  { id: "circle", icon: "circle", label: "Circle", shortcut: "C" },
  { id: "ellipse", icon: "ellipse", label: "Ellipse", shortcut: "O" },
  { id: "arc", icon: "arc", label: "Arc", shortcut: "A" },
  { id: "arc3p", icon: "arc3p", label: "3-point arc", shortcut: "⇧A" },
  { sep: true },
  { id: "polygon", icon: "polygon", label: "Polygon", shortcut: "G" },
  { id: "slot", icon: "slot", label: "Slot", shortcut: "S" },
  { id: "point", icon: "point", label: "Point", shortcut: "P" },
  { sep: true },
  { id: "dimension", icon: "dimension", label: "Dimension", shortcut: "D" },
  { id: "trim", icon: "trim", label: "Trim", shortcut: "T" },
  { id: "extend", icon: "extend", label: "Extend", shortcut: "⇧T" },
  { id: "mirror", icon: "mirror", label: "Mirror", shortcut: "M" },
  { id: "sketchFillet", icon: "fillet", label: "Fillet", shortcut: "F" },
  { id: "sketchOffset", icon: "offset", label: "Offset", shortcut: "⇧O" },
];
