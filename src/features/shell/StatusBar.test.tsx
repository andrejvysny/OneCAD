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
  /*
   * FURTHER AMENDED (Sketcher UX hardening, P1/P2). A body/face selection has
   * no DOF at all — the row used to render a meaningless "DOF: —" placeholder
   * for that case; the whole segment (separator included) is now simply
   * absent instead.
   */
  it("shows the selected sketch's own DOF, and nothing for a body, plus the mono read-out", () => {
    renderStatusBar();
    expect(screen.getByText("DOF: 3")).toBeInTheDocument();
    expect(screen.getByText(/273\.00/)).toBeInTheDocument();

    act(() => selectionStore.getState().set([{ kind: "body", id: "body1" }]));
    expect(screen.queryByText(/^DOF:/)).toBeNull();
  });

  /*
   * A face has no degrees of freedom. The old gate was `sel.kind !== "body"`,
   * so selecting one lit the read-out up and printed a stale sketch's number.
   */
  it("shows no DOF segment for a face selection in model mode", () => {
    renderStatusBar();
    act(() =>
      selectionStore.getState().set([
        { kind: "face", id: "body1#f:0", bodyId: "body1", topoKey: "f:0", elementId: "el_top" },
      ]),
    );
    expect(screen.queryByText(/^DOF:/)).toBeNull();
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
    expect(screen.queryByText(/^DOF:/)).toBeNull();
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

  /*
   * `key={statusHint.message}` alone never remounts a REPEATED identical error
   * (React reuses the node when the key doesn't change), so the one-shot pulse
   * only ever played once no matter how many times the same failure fired.
   * `statusHintSeq` (viewportStore) bumps on every `setStatusHint` call, so the
   * composite key remounts the node even when the message text is unchanged.
   */
  it("re-triggers the error pulse for the SAME message fired twice", () => {
    renderStatusBar();

    act(() =>
      viewportStore.getState().setStatusHint("Extrude failed: boom", { severity: "error", sticky: true }),
    );
    const first = screen.getByTestId("status-hint");
    expect(first).toHaveClass("hint-error-pulse");

    act(() =>
      viewportStore.getState().setStatusHint("Extrude failed: boom", { severity: "error", sticky: true }),
    );
    const second = screen.getByTestId("status-hint");
    expect(second).toHaveClass("hint-error-pulse");
    expect(second).not.toBe(first); // remounted, not reused — the animation replays
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
   * A2 (Sketcher UX cleanup) + P1 hardening follow-up: FOV/Persp-Ortho are
   * dead controls while sketching (sketch mode force-sets ortho), so both
   * are noise during active editing. DOF no longer duplicates here either —
   * `SketchChromeBar`'s own status pill is the DOF surface while sketching,
   * and its `data-testid="sketch-dof"` sr-only node is now e2e's settle
   * probe (`dofPill` in `e2e/helpers.ts`), so this row dropping its copy
   * doesn't need a DOM contract change on the test side.
   */
  it("hides FOV, Persp/Ortho, AND its own DOF copy while actively editing a sketch", () => {
    renderStatusBar();
    act(() => toolStore.getState().setMode("sketch", "sketch2"));

    expect(screen.queryByTestId("fov")).toBeNull();
    expect(screen.queryByRole("tab", { name: "Ortho" })).toBeNull();
    expect(screen.queryByText(/^DOF:/)).toBeNull();
  });

  /*
   * Design item 11 / audit A8 — the sketch plane's own u,v, not world X/Y/Z
   * (on the XY plane world +Y is the sketch's own u, unmatchable to a typed
   * dimension). `cursorPlaneUV` is written by the same engine writer as
   * `cursor` (ViewportRoot's rAF-coalesced pointermove); a mocked write here
   * stands in for that.
   */
  it("shows the sketch-plane U/V read-out while a sketch session is active, X/Y/Z otherwise", () => {
    renderStatusBar();
    expect(screen.getByText(/X\s+273\.00\s+Y\s+210\.00\s+Z\s+0\.00\s+mm/)).toBeInTheDocument();

    act(() => {
      toolStore.getState().setMode("sketch", "sketch2");
      viewportStore.getState().setCursor({ x: 12, y: 34, z: 0 }, { u: 5.5, v: -2.25 });
    });
    expect(screen.getByText(/U\s+5\.50\s+V\s+-2\.25\s+mm/)).toBeInTheDocument();
    expect(screen.queryByText(/^X\s/)).toBeNull();

    act(() => toolStore.getState().setMode("model"));
    expect(screen.getByText(/X\s+12\.00\s+Y\s+34\.00\s+Z\s+0\.00\s+mm/)).toBeInTheDocument();
  });

  it("colors DOF neutrally, never as a warning, in model mode", () => {
    renderStatusBar();
    expect(screen.getByText("DOF: 3")).toHaveClass("text-dof-neutral");
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
