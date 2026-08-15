import { describe, it, expect, beforeEach, afterEach } from "vitest";
import { render, screen, act } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { StatusBar } from "./StatusBar";
import { createPlatform, PlatformProvider } from "@/platform";
import { selectionStore } from "@/stores/selectionStore";
import { viewportStore } from "@/stores/viewportStore";
import { settingsStore } from "@/stores/settingsStore";
import { documentStore, seedMockDocument } from "@/stores/documentStore";
import { toolStore } from "@/stores/toolStore";
import { resetStores } from "@/test/resetStores";

// WP-1.6: StatusBar now hosts `<SlotHost slot={Slots.StatusSection}/>`, which
// needs a `<PlatformProvider>` ancestor (`usePlatform()` throws without one) —
// a bare platform with zero registered panels is enough for a StatusBar test,
// same minimal-fixture shape `reference.test.tsx` uses.
function renderStatusBar() {
  return render(
    <PlatformProvider platform={createPlatform()}>
      <StatusBar />
    </PlatformProvider>,
  );
}

describe("StatusBar", () => {
  beforeEach(() => resetStores());

  /*
   * AMENDED (LGU-1 WP-A, F1's neighbour). This asserted "DOF: 0" for the
   * default selection — which is `sketch2`, a sketch the document registry
   * reports at DOF 3. The zero came from `viewportStore.dofBadge`, which only
   * the LIVE sketch session ever writes, so in model mode it was null and the
   * bar rendered a confident 0 about a sketch it was not reading. The read-out
   * now takes the selected sketch's own number.
   */
  it("shows the selected sketch's own DOF, — for a body, and the mono read-out", () => {
    renderStatusBar();
    expect(screen.getByText("DOF: 3")).toBeInTheDocument();
    expect(screen.getByText(/273\.00/)).toBeInTheDocument();

    act(() => selectionStore.getState().set([{ kind: "body", id: "body1" }]));
    expect(screen.getByText("DOF: —")).toBeInTheDocument();
  });

  /*
   * A face has no degrees of freedom. The old gate was `sel.kind !== "body"`,
   * so selecting one lit the read-out up and printed a stale sketch's number.
   */
  it("shows no DOF for a face selection in model mode", () => {
    renderStatusBar();
    act(() =>
      selectionStore.getState().set([
        { kind: "face", id: "body1#f:0", bodyId: "body1", topoKey: "f:0", elementId: "el_top" },
      ]),
    );
    expect(screen.getByText("DOF: —")).toBeInTheDocument();
  });

  /*
   * The other half of the same defect: a leftover `dofBadge` from the last
   * edited sketch must not be reported beside an unrelated model selection.
   */
  it("ignores a stale dofBadge once the sketch session is over", () => {
    renderStatusBar();
    act(() => {
      viewportStore.setState({ dofBadge: 7 });
      selectionStore.getState().set([{ kind: "body", id: "body1" }]);
    });
    expect(screen.getByText("DOF: —")).toBeInTheDocument();
    expect(screen.queryByText("DOF: 7")).toBeNull();
  });

  it("styles an error hint red and an info hint dimmed", () => {
    renderStatusBar();

    act(() =>
      viewportStore.getState().setStatusHint("Extrude failed: boom", { severity: "error", sticky: true }),
    );
    expect(screen.getByText("Extrude failed: boom")).toHaveClass("text-traffic-close");

    act(() => viewportStore.getState().setStatusHint("Select a plane to start", { sticky: true }));
    const info = screen.getByText("Select a plane to start");
    expect(info).toHaveClass("text-ink-5");
    expect(info).not.toHaveClass("text-traffic-close");
  });

  it("toggles projection and dims FOV in ortho", async () => {
    const user = userEvent.setup();
    renderStatusBar();

    expect(screen.getByTestId("fov")).toHaveStyle({ opacity: "1" });
    await user.click(screen.getByRole("tab", { name: "Ortho" }));
    expect(screen.getByTestId("fov")).toHaveStyle({ opacity: "0.35" });
    await user.click(screen.getByRole("tab", { name: "Persp" }));
    expect(screen.getByTestId("fov")).toHaveStyle({ opacity: "1" });
  });

  /*
   * WP-C2: the store holds mm; the row is display-only. The unit is named once
   * at the end of the row rather than on every axis, and the columns keep their
   * fixed width so the read-out does not jitter as the pointer moves.
   */
  it("renders the cursor read-out in the display unit, named once", () => {
    renderStatusBar();
    // resetStores() seeds cursor = (273, 210, 0) mm at the mm default.
    expect(screen.getByText(/X\s+273\.00\s+Y\s+210\.00\s+Z\s+0\.00\s+mm/)).toBeInTheDocument();

    act(() => settingsStore.getState().setDisplayUnit("cm"));
    expect(screen.getByText(/X\s+27\.300\s+Y\s+21\.000\s+Z\s+0\.000\s+cm/)).toBeInTheDocument();

    act(() => settingsStore.getState().setDisplayUnit("in"));
    expect(screen.getByText(/10\.748.*in/)).toBeInTheDocument();

    act(() => settingsStore.getState().setDisplayUnit("mm"));
    expect(screen.getByText(/273\.00\s.*mm/)).toBeInTheDocument();
  });

  /*
   * A2 (Sketcher UX cleanup): FOV/Persp-Ortho are dead controls while sketching
   * (sketch mode force-sets ortho), so both are noise during active editing.
   * DOF stays visible in EVERY mode, deliberately — e2e's `dofPill`/
   * `clickAtAwaitingDofChange` poll this exact segment as a settle gate
   * across ~47 specs during active sketch editing; only its color-token
   * duplication was the real bug (see the tone test below), not its
   * visibility.
   */
  it("hides FOV and Persp/Ortho while actively editing a sketch, but keeps DOF", () => {
    renderStatusBar();
    act(() => toolStore.getState().setMode("sketch", "sketch2"));

    expect(screen.queryByTestId("fov")).toBeNull();
    expect(screen.queryByRole("tab", { name: "Ortho" })).toBeNull();
    expect(screen.getByText(/^DOF: /)).toBeInTheDocument();
  });

  it("colors DOF neutrally, never as a warning, in both modes", () => {
    renderStatusBar();
    expect(screen.getByText("DOF: 3")).toHaveClass("text-dof-neutral");

    act(() => toolStore.getState().setMode("sketch", "sketch2"));
    expect(screen.getByText(/^DOF: /)).toHaveClass("text-dof-neutral");
  });

  it("still shows DOF and FOV for a model-mode sketch-row selection", () => {
    renderStatusBar();
    expect(toolStore.getState().mode).toBe("model");
    expect(screen.getByText("DOF: 3")).toBeInTheDocument();
    expect(screen.getByTestId("fov")).toBeInTheDocument();
  });
});

/*
 * H7b — the regen busy indicator. The counter is transport state (regen-started /
 * regen-finished on the real lane, the same transitions around the mock's apply),
 * so it is asserted through the store rather than through a client.
 */
describe("StatusBar regen busy (H7b)", () => {
  afterEach(() => act(() => documentStore.getState().regenIdle()));

  it("shows Rebuilding… only while a regen is in flight", () => {
    renderStatusBar();
    expect(screen.queryByTestId("regen-busy")).toBeNull();

    act(() => documentStore.getState().regenStarted());
    expect(screen.getByTestId("regen-busy")).toHaveTextContent("Rebuilding…");

    act(() => documentStore.getState().regenSettled());
    expect(screen.queryByTestId("regen-busy")).toBeNull();
  });

  it("stays up across OVERLAPPING regens and cannot go negative", () => {
    renderStatusBar();
    act(() => {
      documentStore.getState().regenStarted();
      documentStore.getState().regenStarted();
    });
    act(() => documentStore.getState().regenSettled());
    // One still in flight.
    expect(screen.getByTestId("regen-busy")).toBeInTheDocument();
    act(() => documentStore.getState().regenSettled());
    expect(screen.queryByTestId("regen-busy")).toBeNull();

    // A no-op regen finishes without ever starting — the clamp keeps the counter
    // at zero instead of driving it negative and sticking the indicator off.
    act(() => documentStore.getState().regenSettled());
    expect(documentStore.getState().regenBusy).toBe(0);
    act(() => documentStore.getState().regenStarted());
    expect(screen.getByTestId("regen-busy")).toBeInTheDocument();
  });

  it("a projection snapshot landing mid-regen does not clear the indicator", () => {
    renderStatusBar();
    act(() => documentStore.getState().regenStarted());
    act(() => documentStore.getState().applySnapshot(seedMockDocument()));
    expect(screen.getByTestId("regen-busy")).toBeInTheDocument();
  });
});

describe("StatusBar — settings", () => {
  beforeEach(() => resetStores());

  /*
   * Settings is APPLICATION chrome. It was mounted from `ModelTreePanel`, which
   * made "which application-wide preferences exist" a model-tree responsibility
   * — the kind of small cross-cutting dependency that quietly undoes module
   * boundaries. `ModelTreePanel.test.tsx` asserts it is no longer there.
   */
  it("opens and closes the settings dialog", async () => {
    const user = userEvent.setup();
    renderStatusBar();

    await user.click(screen.getByRole("button", { name: "Open settings" }));
    expect(screen.getByRole("dialog", { name: "Settings" })).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Close settings" }));
    expect(screen.queryByRole("dialog", { name: "Settings" })).toBeNull();
  });
});
