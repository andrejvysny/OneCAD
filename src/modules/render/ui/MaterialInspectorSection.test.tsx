/*
 * The inspector's Material sections.
 *
 * Rendered BARE, without a platform: a `ContributionComponent` takes no props
 * and reads the inspector context itself, so what a test has to arrange is the
 * selection — which is what the host would have arranged too. The registration
 * that decides WHEN these mount is covered in `../register.test.ts`, and their
 * ORDER against modeling's sections by the frozen inspector contract.
 */
import { describe, it, expect, beforeEach, afterEach } from "vitest";
import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import { mockClient, getMockLatency, setMockLatency } from "@/ipc/mockClient";
import { selectionStore } from "@/stores/selectionStore";
import { toolStore } from "@/stores/toolStore";
import { settleUntil } from "@/test/settle";

import { createMaterial } from "../model/material";
import { renderStore, setRenderDocumentStateService } from "../store/renderStore";
import { renderDialogStore } from "./dialogStore";
import {
  MaterialBodySection,
  MaterialFaceSection,
  bodyIdFromFaceEntity,
} from "./MaterialInspectorSection";

const STEEL = createMaterial("Steel");
const BRASS = createMaterial("Brass");
const realLatency = getMockLatency();

const settle = {
  turn: () => act(async () => void (await new Promise((r) => setTimeout(r, 0)))),
};

function selectBody(id = "body1") {
  act(() => {
    toolStore.getState().setMode("model");
    selectionStore.getState().set([{ kind: "body", id }]);
  });
}

function selectFace(bodyId = "body1", topoKey = "f:0", elementId = "el_top") {
  act(() => {
    toolStore.getState().setMode("model");
    selectionStore
      .getState()
      .set([{ kind: "face", id: `${bodyId}#${topoKey}`, bodyId, topoKey, elementId }]);
  });
}

beforeEach(async () => {
  setMockLatency(0);
  setRenderDocumentStateService(null);
  renderStore.getState().reset();
  renderDialogStore.getState().reset();
  await mockClient.closeDocument();
  await renderStore.getState().upsertMaterial(STEEL);
  await renderStore.getState().upsertMaterial(BRASS);
});

afterEach(() => {
  setMockLatency(realLatency);
  renderStore.getState().reset();
  renderDialogStore.getState().reset();
});

describe("bodyIdFromFaceEntity", () => {
  it("recovers the owning body from the composite face ref", () => {
    expect(bodyIdFromFaceEntity("body1#f:22")).toBe("body1");
    // Nothing composite ⇒ no guess. The inherited line then reads "None",
    // which is honest; nothing is ever WRITTEN against a parsed body id.
    expect(bodyIdFromFaceEntity("body1")).toBeNull();
    expect(bodyIdFromFaceEntity("")).toBeNull();
  });
});

describe("MaterialBodySection", () => {
  it("shows the body's assigned material", async () => {
    await renderStore.getState().assignBody("body1", STEEL.id);
    selectBody();
    render(<MaterialBodySection />);

    expect(screen.getByLabelText("Body material")).toHaveValue(STEEL.id);
  });

  it("shows None for an unassigned body, and offers every material", () => {
    selectBody();
    render(<MaterialBodySection />);

    const select = screen.getByLabelText("Body material");
    expect(select).toHaveValue("");
    expect(screen.getByRole("option", { name: "None" })).toBeInTheDocument();
    expect(screen.getByRole("option", { name: "Steel" })).toBeInTheDocument();
    expect(screen.getByRole("option", { name: "Brass" })).toBeInTheDocument();
  });

  it("choosing a material assigns it to the body", async () => {
    selectBody();
    render(<MaterialBodySection />);

    await userEvent.selectOptions(screen.getByLabelText("Body material"), BRASS.id);
    await settleUntil(
      () => expect(renderStore.getState().state.assignments.bodies.body1).toBe(BRASS.id),
      settle,
    );
  });

  it("choosing None unassigns it", async () => {
    await renderStore.getState().assignBody("body1", STEEL.id);
    selectBody();
    render(<MaterialBodySection />);

    await userEvent.selectOptions(screen.getByLabelText("Body material"), "");
    await settleUntil(
      () => expect(renderStore.getState().state.assignments.bodies.body1).toBeUndefined(),
      settle,
    );
  });

  it("a body with bound overrides raises the keep/replace question instead of assigning", async () => {
    act(() => renderStore.getState().reportBoundFaceOverrides("body1", ["el_top", "el_side"]));
    selectBody();
    render(<MaterialBodySection />);

    await userEvent.selectOptions(screen.getByLabelText("Body material"), BRASS.id);
    await settleUntil(
      () => expect(renderDialogStore.getState().override).not.toBeNull(),
      settle,
    );
    // Nothing written until the question is answered.
    expect(renderStore.getState().state.assignments.bodies.body1).toBeUndefined();

    act(() => renderDialogStore.getState().answerOverride("keep"));
    await settleUntil(
      () => expect(renderStore.getState().state.assignments.bodies.body1).toBe(BRASS.id),
      settle,
    );
  });

  it("counts the bound face overrides and clears them on demand", async () => {
    await renderStore.getState().assignFace("el_top", BRASS.id);
    await renderStore.getState().assignFace("el_side", BRASS.id);
    act(() => renderStore.getState().reportBoundFaceOverrides("body1", ["el_top", "el_side"]));
    selectBody();
    render(<MaterialBodySection />);

    expect(screen.getByTestId("material-override-count")).toHaveTextContent("2 face overrides");
    await userEvent.click(screen.getByTestId("material-override-clear"));
    await settleUntil(
      () => expect(renderStore.getState().state.assignments.faces).toEqual({}),
      settle,
    );
  });

  it("renders no override line when the body has none", () => {
    selectBody();
    render(<MaterialBodySection />);
    expect(screen.queryByTestId("material-override-count")).toBeNull();
  });
});

describe("MaterialFaceSection", () => {
  it("reports the material INHERITED from the body", async () => {
    await renderStore.getState().assignBody("body1", STEEL.id);
    selectFace();
    render(<MaterialFaceSection />);

    expect(screen.getByTestId("material-face-effective")).toHaveTextContent("From body: Steel");
    // The dropdown shows the face's OWN override, which is none — the inherited
    // value is not an override and must not look like one.
    expect(screen.getByLabelText("Face override")).toHaveValue("");
  });

  it("an override WINS over the body's material and says so", async () => {
    await renderStore.getState().assignBody("body1", STEEL.id);
    await renderStore.getState().assignFace("el_top", BRASS.id);
    selectFace();
    render(<MaterialFaceSection />);

    expect(screen.getByTestId("material-face-effective")).toHaveTextContent("Override: Brass");
    expect(screen.getByLabelText("Face override")).toHaveValue(BRASS.id);
  });

  it("reads None when neither the face nor its body carries one", () => {
    selectFace();
    render(<MaterialFaceSection />);
    expect(screen.getByTestId("material-face-effective")).toHaveTextContent("None");
  });

  it("sets and clears the override, and never prompts", async () => {
    act(() => renderStore.getState().reportBoundFaceOverrides("body1", ["el_top"]));
    selectFace();
    render(<MaterialFaceSection />);

    await userEvent.selectOptions(screen.getByLabelText("Face override"), BRASS.id);
    await settleUntil(
      () => expect(renderStore.getState().state.assignments.faces.el_top).toBe(BRASS.id),
      settle,
    );
    // Authoring an override IS the answer to the keep/replace question, so
    // asking it here would be asking the user to confirm what they just did.
    expect(renderDialogStore.getState().override).toBeNull();

    await userEvent.selectOptions(screen.getByLabelText("Face override"), "");
    await settleUntil(
      () => expect(renderStore.getState().state.assignments.faces.el_top).toBeUndefined(),
      settle,
    );
  });

  it("renders nothing for a face with no promoted ElementId", () => {
    act(() => {
      toolStore.getState().setMode("model");
      selectionStore
        .getState()
        .set([{ kind: "face", id: "body1#f:0", bodyId: "body1", topoKey: "f:0" }]);
    });
    const { container } = render(<MaterialFaceSection />);
    expect(container.textContent).toBe("");
  });
});
