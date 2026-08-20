/*
 * ConstraintBadgeLayer — the INTERACTIVE canvas badges (audit A6 / plan item 5).
 *
 * No engine is set, so the overlay-driver registration effect early-returns and
 * the badges render unpositioned — which is exactly the surface under test here
 * (roles, names, selection, hover, tint). Positioning is the driver's own
 * contract and has its own tests (`HtmlOverlayDriver.test.ts`).
 */
import { describe, it, expect, beforeEach } from "vitest";
import { render, screen, act, fireEvent } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { ConstraintBadgeLayer } from "./ConstraintBadgeLayer";
import { toolStore } from "@/stores/toolStore";
import { sketchStore } from "@/stores/sketchStore";
import { settingsStore } from "@/stores/settingsStore";
import { sketchSelectionStore } from "@/stores/sketchSelectionStore";
import { planeFor } from "@/ipc/mockSketch";
import { resetStores } from "@/test/resetStores";
import type { SketchConstraint, SketchEntity } from "@/ipc/types";

const entities: SketchEntity[] = [
  { id: "e1", type: "Line", p0: [0, 0], p1: [40, 0] },
  { id: "e2", type: "Line", p0: [0, 10], p1: [40, 12] },
];

const horizontal: SketchConstraint = { id: "c1", type: "Horizontal", entities: ["e1"] };
const parallel: SketchConstraint = { id: "c2", type: "Parallel", entities: ["e1", "e2"] };

// Badges are interactive only while the select tool is active (M3) — every
// pre-existing test in this file exercises that surface, so entering sketch
// mode here arms select by default. The dedicated tool-gating tests below
// switch away from it explicitly.
function enterSketch(constraints: SketchConstraint[]) {
  act(() => {
    toolStore.getState().setMode("sketch", undefined, { tool: "select" });
    sketchStore.getState().setSession({
      sketchId: "sk",
      plane: planeFor("XY"),
      entities,
      constraints,
      dof: 4,
      status: "UnderConstrained",
    });
  });
}

const badge = (id: string) => screen.getByTestId(`constraint-badge-${id}`);

describe("ConstraintBadgeLayer", () => {
  beforeEach(() => {
    resetStores();
    sketchSelectionStore.getState().clear();
  });

  it("renders nothing outside sketch mode, or with the chip preference off", () => {
    render(<ConstraintBadgeLayer />);
    expect(screen.queryByTestId("constraint-badges")).toBeNull();

    enterSketch([horizontal]);
    expect(screen.getByTestId("constraint-badges")).toBeInTheDocument();

    act(() => settingsStore.getState().setShow("constraintChips", false));
    expect(screen.queryByTestId("constraint-badges")).toBeNull();
  });

  it("the container stays inert while each badge takes pointer events itself", () => {
    render(<ConstraintBadgeLayer />);
    enterSketch([horizontal]);
    // The container covers the WHOLE viewport — it must never eat a click
    // aimed at geometry.
    expect(screen.getByTestId("constraint-badges").className).toMatch(/pointer-events-none/);
    expect(badge("c1").className).toMatch(/pointer-events-auto/);
  });

  it("each glyph badge is a focusable button named for its constraint", () => {
    render(<ConstraintBadgeLayer />);
    enterSketch([horizontal, parallel]);

    const b = screen.getByRole("button", { name: "Horizontal" });
    expect(b).toBe(badge("c1"));
    expect(b).toHaveAttribute("title", "Horizontal");
    expect(b.tagName).toBe("BUTTON"); // keyboard-reachable, Enter/Space activate
    expect(screen.getByRole("button", { name: "Parallel" })).toBe(badge("c2"));
  });

  it("clicking a badge selects that constraint and drops the entity selection", async () => {
    const user = userEvent.setup();
    render(<ConstraintBadgeLayer />);
    enterSketch([horizontal, parallel]);
    act(() => sketchSelectionStore.getState().set([{ entityId: "e1" }]));

    await user.click(badge("c1"));

    expect(sketchSelectionStore.getState().selectedConstraintId).toBe("c1");
    expect(sketchSelectionStore.getState().selected).toEqual([]);
    expect(badge("c1")).toHaveAttribute("data-selected", "true");
    expect(badge("c1")).toHaveAttribute("aria-pressed", "true");
    expect(badge("c2")).not.toHaveAttribute("data-selected");
  });

  it("clicking the selected badge again deselects it", async () => {
    const user = userEvent.setup();
    render(<ConstraintBadgeLayer />);
    enterSketch([horizontal]);

    await user.click(badge("c1"));
    await user.click(badge("c1"));
    expect(sketchSelectionStore.getState().selectedConstraintId).toBeNull();
  });

  it("selecting an entity clears the badge selection", async () => {
    const user = userEvent.setup();
    render(<ConstraintBadgeLayer />);
    enterSketch([horizontal]);
    await user.click(badge("c1"));

    act(() => sketchSelectionStore.getState().set([{ entityId: "e2" }]));
    expect(sketchSelectionStore.getState().selectedConstraintId).toBeNull();
    expect(badge("c1")).not.toHaveAttribute("data-selected");
  });

  it("a CONFLICTING badge keeps its red tint even when selected", async () => {
    const user = userEvent.setup();
    render(<ConstraintBadgeLayer />);
    enterSketch([horizontal]);
    act(() => sketchStore.getState().setConflicting(["c1"]));

    expect(badge("c1").className).toMatch(/text-traffic-close/);
    await user.click(badge("c1"));
    // Selected, but the solver's complaint still owns the color.
    expect(sketchSelectionStore.getState().selectedConstraintId).toBe("c1");
    expect(badge("c1").className).toMatch(/text-traffic-close/);
    expect(badge("c1").className).not.toMatch(/ring-accent/);
  });

  it("hovering a badge publishes the constraint hover (canvas cross-highlight)", () => {
    render(<ConstraintBadgeLayer />);
    enterSketch([horizontal]);

    fireEvent.mouseEnter(badge("c1"));
    expect(sketchSelectionStore.getState().constraintHover).toBe("c1");
    fireEvent.mouseLeave(badge("c1"));
    expect(sketchSelectionStore.getState().constraintHover).toBeNull();
  });

  it("hovering an ENTITY tints the badges that reference it (the reverse link)", () => {
    render(<ConstraintBadgeLayer />);
    enterSketch([horizontal, parallel]);
    const idle = badge("c1").className;

    act(() => sketchSelectionStore.getState().setHover({ entityId: "e2" }));
    // Parallel names e2; Horizontal does not.
    expect(badge("c2").className).not.toBe(idle);
    expect(badge("c2").className).toMatch(/text-accent/);
    expect(badge("c1").className).toBe(idle);

    act(() => sketchSelectionStore.getState().setHover(null));
    expect(badge("c1").className).toBe(idle);
  });

  it("a NEWLY authored badge flashes once; the ones already on screen do not", () => {
    render(<ConstraintBadgeLayer />);
    enterSketch([horizontal]);
    // Entering a sketch that already has constraints must not set the whole
    // canvas blinking.
    expect(badge("c1").className).not.toMatch(/badge-mount-flash/);

    enterSketch([horizontal, parallel]);
    expect(badge("c2").className).toMatch(/badge-mount-flash/);
    expect(badge("c1").className).not.toMatch(/badge-mount-flash/);
  });
});

const distance: SketchConstraint = {
  id: "d1",
  type: "Distance",
  entities: ["e1", "e1"],
  positions: ["Start", "End"],
  value: 40,
};

// M3: a badge must never steal a pointer event a drawing gesture needs, so it
// is interactive ONLY while the select tool is armed.
describe("ConstraintBadgeLayer — tool gating (M3)", () => {
  beforeEach(() => {
    resetStores();
    sketchSelectionStore.getState().clear();
  });

  it("a glyph badge is an inert span outside the select tool — no click, no hover", () => {
    render(<ConstraintBadgeLayer />);
    enterSketch([horizontal]);
    act(() => toolStore.getState().setTool("line"));

    const b = badge("c1");
    expect(b.tagName).toBe("SPAN");
    expect(b.className).toMatch(/pointer-events-none/);
    expect(b).not.toHaveAttribute("aria-pressed");

    fireEvent.click(b);
    expect(sketchSelectionStore.getState().selectedConstraintId).toBeNull();
    fireEvent.mouseEnter(b);
    expect(sketchSelectionStore.getState().constraintHover).toBeNull();
  });

  it("re-arming the select tool restores the interactive button", async () => {
    const user = userEvent.setup();
    render(<ConstraintBadgeLayer />);
    enterSketch([horizontal]);
    act(() => toolStore.getState().setTool("line"));
    expect(badge("c1").tagName).toBe("SPAN");

    act(() => toolStore.getState().setTool("select"));
    expect(badge("c1").tagName).toBe("BUTTON");
    await user.click(badge("c1"));
    expect(sketchSelectionStore.getState().selectedConstraintId).toBe("c1");
  });

  // M3 follow-up: gating the dimensional chip too broke `e2e/dimension-edit.spec.ts`
  // (draws with a shape tool, edits the just-committed chip with no tool switch in
  // between) — a value pill is an unambiguous click target in ANY tool (Shapr3D
  // convention), unlike the 20px glyph circles that sit directly on geometry.
  it("a dimensional chip stays editable in EVERY tool, unlike a glyph badge", () => {
    render(<ConstraintBadgeLayer />);
    enterSketch([distance]);
    expect(screen.getByRole("textbox", { name: "Dimension value" })).toHaveValue("40");

    act(() => toolStore.getState().setTool("line"));
    expect(screen.getByRole("textbox", { name: "Dimension value" })).toHaveValue("40");

    act(() => toolStore.getState().setTool("circle"));
    expect(screen.getByRole("textbox", { name: "Dimension value" })).toHaveValue("40");
  });
});

// SKETCH-ON-FACE W2: a machine `Fixed` pin on locked reference geometry is
// selectable-but-not-deletable if left interactive (Delete on it is a silent
// no-op) — it must never be a live badge, in any tool, select included.
describe("ConstraintBadgeLayer — machine Fixed pins on locked geometry stay inert", () => {
  beforeEach(() => {
    resetStores();
    sketchSelectionStore.getState().clear();
  });

  const lockedRef: SketchEntity = { id: "ref1", type: "Line", p0: [0, 0], p1: [40, 0], referenceLocked: true };
  const machineFixed: SketchConstraint = { id: "fx1", type: "Fixed", entities: ["ref1"], positions: ["Start"] };

  function enterSketchWithLocked(constraints: SketchConstraint[]) {
    act(() => {
      toolStore.getState().setMode("sketch", undefined, { tool: "select" });
      sketchStore.getState().setSession({
        sketchId: "sk-locked",
        plane: planeFor("XY"),
        entities: [lockedRef, ...entities],
        constraints,
        dof: 0,
        status: "FullyConstrained",
      });
    });
  }

  it("renders as inert even while the select tool is active — the ONE tool where every other badge is live", async () => {
    const user = userEvent.setup();
    render(<ConstraintBadgeLayer />);
    enterSketchWithLocked([machineFixed, horizontal]);

    // The co-existing USER constraint on unlocked geometry stays fully live —
    // this is not a blanket "select tool disabled" regression.
    await user.click(badge("c1"));
    expect(sketchSelectionStore.getState().selectedConstraintId).toBe("c1");

    const pin = badge("fx1");
    expect(pin.tagName).toBe("SPAN");
    expect(pin.className).toMatch(/pointer-events-none/);
    await user.click(pin);
    expect(sketchSelectionStore.getState().selectedConstraintId).toBe("c1"); // unchanged
    fireEvent.mouseEnter(pin);
    expect(sketchSelectionStore.getState().constraintHover).toBeNull();
  });
});
