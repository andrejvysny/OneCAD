/*
 * ADR-0008 promised the id maps stay complete in both directions. This is the
 * enforcement: the maps, the descriptor tables and the store unions must name
 * exactly the same tools, or one representation has drifted from another.
 */
import { describe, it, expect } from "vitest";
import { ModelingModelTools, ModelingSketchTools, ModelingCommands } from "./ids";
import { MODEL_TOOL_DESCRIPTORS, SKETCH_TOOL_DESCRIPTORS } from "./tools";
import { MODEL_TOOLS_CONTRACT, SKETCH_TOOLS_CONTRACT } from "@/test/contracts/toolbarContract";
import { isSeparator } from "@/features/toolbar/toolbarConfig";
import { isOwnerIdShape } from "@/platform";

const contractTools = (entries: typeof MODEL_TOOLS_CONTRACT): string[] =>
  entries.filter((e) => !isSeparator(e)).map((e) => (isSeparator(e) ? "" : e.id));

describe("modeling id maps", () => {
  it("cover exactly the tools the shipped toolbar contains", () => {
    expect(Object.keys(ModelingModelTools).sort()).toEqual(
      contractTools(MODEL_TOOLS_CONTRACT).sort(),
    );
    expect(Object.keys(ModelingSketchTools).sort()).toEqual(
      contractTools(SKETCH_TOOLS_CONTRACT).sort(),
    );
  });

  it("agree with the descriptor tables", () => {
    expect(MODEL_TOOL_DESCRIPTORS.map((d) => d.tool).sort()).toEqual(
      Object.keys(ModelingModelTools).sort(),
    );
    expect(SKETCH_TOOL_DESCRIPTORS.map((d) => d.tool).sort()).toEqual(
      Object.keys(ModelingSketchTools).sort(),
    );
  });

  it("are unique across both scopes", () => {
    const all = [
      ...Object.values(ModelingModelTools),
      ...Object.values(ModelingSketchTools),
      ...Object.values(ModelingCommands),
    ];
    expect(new Set(all).size).toBe(all.length);
  });

  it("live under the module's namespace", () => {
    for (const id of [
      ...Object.values(ModelingModelTools),
      ...Object.values(ModelingSketchTools),
      ...Object.values(ModelingCommands),
    ]) {
      expect(id.startsWith("onecad.modeling."), id).toBe(true);
    }
    expect(isOwnerIdShape("onecad.modeling")).toBe(true);
  });

  it("keeps priorities strictly increasing inside each table", () => {
    // The toolbar derives separators from group CHANGES, so a group must be one
    // contiguous run — a re-used group id later in the table would draw a stray
    // separator and silently reorder the palette.
    for (const table of [MODEL_TOOL_DESCRIPTORS, SKETCH_TOOL_DESCRIPTORS]) {
      const priorities = table.map((d) => d.priority);
      expect([...priorities].sort((a, b) => a - b)).toEqual(priorities);

      const seen = new Set<string>();
      let current: string | undefined;
      for (const d of table) {
        if (d.group !== current) {
          expect(seen.has(d.group), `group ${d.group} is not contiguous`).toBe(false);
          seen.add(d.group);
          current = d.group;
        }
      }
    }
  });
});
