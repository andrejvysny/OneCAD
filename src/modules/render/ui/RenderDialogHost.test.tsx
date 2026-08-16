/*
 * The three dialogs the overlay host mounts, and the one rule they share: a
 * dialog that is not open renders NOTHING (a `Slots.ShellOverlay` contribution
 * is mounted unconditionally, so "closed" has to mean invisible, not hidden).
 */
import { describe, it, expect, beforeEach, afterEach } from "vitest";
import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import { mockClient, getMockLatency, setMockLatency } from "@/ipc/mockClient";
import { settleUntil } from "@/test/settle";

import { createMaterial } from "../model/material";
import { renderStore, setRenderDocumentStateService } from "../store/renderStore";
import { renderDialogStore } from "./dialogStore";
import { RenderDialogHost } from "./RenderDialogHost";
import { applyDraft, draftFrom } from "./MaterialEditorSheet";

const STEEL = createMaterial("Steel", { base: { base_metalness: 1 } });
const GLASS = createMaterial("Clear Glass", {
  base: { base_color: [1, 1, 1] },
  transmission: { transmission_weight: 1 },
});
const realLatency = getMockLatency();

const settle = {
  turn: () => act(async () => void (await new Promise((r) => setTimeout(r, 0)))),
};

beforeEach(async () => {
  setMockLatency(0);
  setRenderDocumentStateService(null);
  renderStore.getState().reset();
  renderDialogStore.getState().reset();
  await mockClient.closeDocument();
  await renderStore.getState().upsertMaterial(STEEL);
  await renderStore.getState().upsertMaterial(GLASS);
});

afterEach(() => {
  setMockLatency(realLatency);
  renderStore.getState().reset();
  renderDialogStore.getState().reset();
});

describe("RenderDialogHost", () => {
  it("renders nothing until something is opened", () => {
    render(<RenderDialogHost />);
    expect(screen.queryByRole("dialog")).toBeNull();
  });
});

describe("OverrideKeepDialog", () => {
  function open() {
    return renderDialogStore.getState().requestOverrideChoice({
      bodyId: "body1",
      bodyLabel: "Bracket",
      materialId: STEEL.id,
      materialName: "Steel",
      overrideElementIds: ["el_a", "el_b"],
    });
  }

  it("names the body, the count and the material being assigned", async () => {
    render(<RenderDialogHost />);
    const answered = open();

    const dialog = await screen.findByTestId("material-override-dialog");
    expect(dialog).toHaveTextContent("Bracket has 2 face overrides");
    expect(dialog).toHaveTextContent("Steel");

    act(() => renderDialogStore.getState().answerOverride("cancel"));
    await expect(answered).resolves.toBe("cancel");
  });

  it("resolves keep / replace from their buttons", async () => {
    render(<RenderDialogHost />);

    const kept = open();
    await userEvent.click(await screen.findByTestId("material-override-keep"));
    await expect(kept).resolves.toBe("keep");

    const replaced = open();
    await userEvent.click(await screen.findByTestId("material-override-replace"));
    await expect(replaced).resolves.toBe("replace");
  });

  it("Esc cancels", async () => {
    render(<RenderDialogHost />);
    const answered = open();
    await screen.findByTestId("material-override-dialog");

    await userEvent.keyboard("{Escape}");
    await expect(answered).resolves.toBe("cancel");
  });
});

describe("AssignMaterialDialog", () => {
  it("assigns the picked material to the row it was opened on", async () => {
    render(<RenderDialogHost />);
    act(() => renderDialogStore.getState().openAssign({ bodyId: "body7", bodyLabel: "Bracket" }));

    await userEvent.click(await screen.findByTestId(`assign-material-${STEEL.id}`));
    await settleUntil(
      () => expect(renderStore.getState().state.assignments.bodies.body7).toBe(STEEL.id),
      settle,
    );
    // Closed before assigning, so the override prompt (when there is one) is
    // never raised behind this dialog's own scrim.
    expect(renderDialogStore.getState().assign).toBeNull();
  });

  it("None unassigns", async () => {
    await renderStore.getState().assignBody("body7", STEEL.id);
    render(<RenderDialogHost />);
    act(() => renderDialogStore.getState().openAssign({ bodyId: "body7" }));

    await userEvent.click(await screen.findByTestId("assign-material-none"));
    await settleUntil(
      () => expect(renderStore.getState().state.assignments.bodies.body7).toBeUndefined(),
      settle,
    );
  });
});

describe("MaterialEditorSheet", () => {
  it("commits on Save, not per keystroke", async () => {
    render(<RenderDialogHost />);
    act(() => renderDialogStore.getState().openEditor(STEEL.id));

    const name = await screen.findByLabelText("Material name");
    await userEvent.clear(name);
    await userEvent.type(name, "Stainless");
    // Nothing written yet: a per-keystroke editor would push one undo entry per
    // character and make Ctrl-Z useless on a material.
    expect(renderStore.getState().state.library[STEEL.id]?.name).toBe("Steel");

    await userEvent.click(screen.getByRole("button", { name: "Save" }));
    await settleUntil(
      () => expect(renderStore.getState().state.library[STEEL.id]?.name).toBe("Stainless"),
      settle,
    );
    expect(renderDialogStore.getState().editor).toBeNull();
  });

  it("Cancel discards the draft", async () => {
    render(<RenderDialogHost />);
    act(() => renderDialogStore.getState().openEditor(STEEL.id));

    const name = await screen.findByLabelText("Material name");
    await userEvent.clear(name);
    await userEvent.type(name, "Nope");
    await userEvent.click(screen.getByRole("button", { name: "Cancel" }));

    await settle.turn();
    expect(renderStore.getState().state.library[STEEL.id]?.name).toBe("Steel");
  });

  it("the Coat toggle reveals its parameters", async () => {
    render(<RenderDialogHost />);
    act(() => renderDialogStore.getState().openEditor(STEEL.id));

    await screen.findByTestId("material-editor");
    expect(screen.queryByLabelText("Coat weight")).toBeNull();
    await userEvent.click(screen.getByRole("switch", { name: "Coat" }));
    expect(screen.getByLabelText("Coat weight")).toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: "Save" }));
    await settleUntil(
      () => expect(renderStore.getState().state.library[STEEL.id]?.coat).toBeDefined(),
      settle,
    );
  });
});

describe("applyDraft", () => {
  it("adds a layer when its toggle goes on and REMOVES it when it goes off", () => {
    const withCoat = applyDraft(STEEL, { ...draftFrom(STEEL), coat: true, coatWeight: 0.5 });
    expect(withCoat.coat).toEqual({ coat_weight: 0.5, coat_roughness: 0 });

    // An ABSENT layer is the model's own way of saying "lobe off" — writing
    // `coat_weight: 0` instead would leave the key in the record forever.
    const without = applyDraft(withCoat, { ...draftFrom(withCoat), coat: false });
    expect("coat" in without).toBe(false);
  });

  it("preserves a layer the sheet has no field for", () => {
    // Clear Glass carries a transmission lobe this editor cannot show. Saving an
    // unrelated change must not silently make it opaque.
    const saved = applyDraft(GLASS, { ...draftFrom(GLASS), metalness: 0.25 });
    expect(saved.transmission).toEqual({ transmission_weight: 1 });
    expect(saved.base.base_metalness).toBe(0.25);
    expect(saved.id).toBe(GLASS.id);
  });

  it("keeps the old name rather than accepting a blank one", () => {
    expect(applyDraft(STEEL, { ...draftFrom(STEEL), name: "   " }).name).toBe("Steel");
  });

  it("round-trips a colour through the sRGB control values", () => {
    const draft = draftFrom(GLASS);
    const saved = applyDraft(GLASS, draft);
    for (const [i, channel] of (saved.base.base_color ?? []).entries()) {
      expect(channel).toBeCloseTo((GLASS.base.base_color ?? [])[i], 2);
    }
  });
});
