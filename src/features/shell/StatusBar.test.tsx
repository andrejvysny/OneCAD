import { describe, it, expect, beforeEach, afterEach } from "vitest";
import { render, screen, act } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { StatusBar } from "./StatusBar";
import { createPlatform, PlatformProvider } from "@/platform";
import { selectionStore } from "@/stores/selectionStore";
import { viewportStore } from "@/stores/viewportStore";
import { settingsStore } from "@/stores/settingsStore";
import { documentStore, seedMockDocument } from "@/stores/documentStore";
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

  it("shows DOF for a selected sketch, — for a body, and the mono read-out", () => {
    renderStatusBar();
    expect(screen.getByText("DOF: 0")).toBeInTheDocument();
    expect(screen.getByText(/273\.00/)).toBeInTheDocument();

    act(() => selectionStore.getState().set([{ kind: "body", id: "body1" }]));
    expect(screen.getByText("DOF: —")).toBeInTheDocument();
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
