/*
 * The "Save as Component" dialog (WP-B2): what it sends, what it refuses
 * before a round trip, and what it does with a backend refusal.
 *
 * The last one is the interesting case. Every way this save can fail — a taken
 * id@version, a body that bakes to more than one solid (spec §9) — is
 * something the author fixes right here, so the dialog must stay OPEN and keep
 * their typing. A dialog that closed on failure would throw the work away and
 * leave a bare status hint behind.
 */
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import {
  isValidComponentId,
  isValidVersion,
  suggestedId,
  SaveAsComponentDialog,
} from "./SaveAsComponentDialog";
import type { ClassifyResult, LibraryComponent, NewComponentSpec } from "@/ipc/types";
import { configureAuthoringController } from "@/modules/library/authoringController";
import { cancelAttachmentPick } from "@/modules/library/attachmentPicker";

const saveAsComponent = vi.fn();
const listVariables = vi.fn();
const classifyElement = vi.fn();
const engine = vi.hoisted(() => ({
  probePick: vi.fn(),
  setOrbitSuppressed: vi.fn(),
}));

vi.mock("@/ipc/client", () => ({
  createClient: () => ({
    saveAsComponent: (...args: unknown[]) => saveAsComponent(...args),
    listVariables: () => listVariables(),
  }),
}));
vi.mock("@/viewport/engineBridge", () => ({
  getViewportEngine: () => ({
    captureThumbnail: () => "data:image/png;base64,AAAA",
    probePick: engine.probePick,
    setOrbitSuppressed: engine.setOrbitSuppressed,
  }),
}));

/** A viewport pick landing at `world`, as `ViewportEngine.probePick` reports it. */
function hitAt(world: [number, number, number]) {
  return {
    bodyId: "body_1",
    kind: "face" as const,
    topoKey: "f:22",
    elementId: "el_1",
    distance: 1,
    worldPos: { x: world[0], y: world[1], z: world[2] },
  };
}

const PLANE_TOP: ClassifyResult = {
  kind: "face",
  surfaceType: "plane",
  curveType: "",
  frame: { origin: [0, 0, 10], normal: [0, 0, 1], axis: null, radius: null },
};

const CYLINDER: ClassifyResult = {
  kind: "face",
  surfaceType: "cylinder",
  curveType: "",
  frame: { origin: [4, 4, 0], normal: null, axis: [0, 0, 1], radius: 3 },
};

/** One viewport click at `(x, y)`, as the capture-phase picker sees it. */
async function pickInViewport(): Promise<void> {
  // `MouseEvent`, not `PointerEvent`: jsdom does not implement the latter, and
  // the listener keys off the event TYPE, which is all that matters here.
  window.dispatchEvent(
    new MouseEvent("pointerdown", { clientX: 40, clientY: 40, bubbles: true }),
  );
  await waitFor(() => expect(classifyElement).toHaveBeenCalled());
}

function saved(spec: NewComponentSpec): LibraryComponent {
  return {
    id: spec.id,
    version: spec.version,
    name: spec.name,
    category: spec.category,
    tags: spec.tags,
    sourceKind: "document",
    revision: `sha256:${"0".repeat(64)}`,
    attachments: spec.attachments,
    parameters: {},
  } as LibraryComponent;
}

describe("suggestedId", () => {
  it("slugifies a body name into a namespaced id", () => {
    expect(suggestedId("Bracket Plate")).toBe("mine.bracket-plate");
    expect(suggestedId("  Motor Mount v2 ")).toBe("mine.motor-mount-v2");
  });

  it("is empty when the name carries nothing usable, rather than proposing `mine.`", () => {
    expect(suggestedId("   ")).toBe("");
    expect(suggestedId("···")).toBe("");
  });
});

describe("id and version validation", () => {
  it("requires a namespaced lowercase id — the backend's own rule", () => {
    expect(isValidComponentId("mine.bracket")).toBe(true);
    expect(isValidComponentId("acme.parts.bracket-2")).toBe(true);
    expect(isValidComponentId("bracket")).toBe(false); // not namespaced
    expect(isValidComponentId("Mine.Bracket")).toBe(false); // uppercase
    expect(isValidComponentId("mine.")).toBe(false);
  });

  it("requires a semver triple", () => {
    expect(isValidVersion("1.0.0")).toBe(true);
    expect(isValidVersion("1.0")).toBe(false);
    expect(isValidVersion("v1.0.0")).toBe(false);
  });
});

describe("SaveAsComponentDialog", () => {
  beforeEach(() => {
    saveAsComponent.mockReset();
    saveAsComponent.mockImplementation((_body: string, spec: NewComponentSpec) =>
      Promise.resolve(saved(spec)),
    );
    listVariables.mockReset();
    listVariables.mockResolvedValue([]);
    classifyElement.mockReset();
    engine.probePick.mockReset();
    engine.setOrbitSuppressed.mockReset();
    configureAuthoringController({ geometryQuery: { classifyElement } });
  });

  afterEach(() => {
    cancelAttachmentPick();
    configureAuthoringController(null);
  });

  it("sends the body, the typed identity, and a viewport thumbnail", async () => {
    const onClose = vi.fn();
    render(<SaveAsComponentDialog bodyId="body_1" bodyName="Bracket Plate" onClose={onClose} />);
    const user = userEvent.setup();

    await user.type(screen.getByTestId("save-as-component-tags"), "printed, jig");
    await user.click(screen.getByTestId("save-as-component-commit"));

    await waitFor(() => expect(saveAsComponent).toHaveBeenCalledTimes(1));
    const [bodyId, spec, preview] = saveAsComponent.mock.calls[0];
    expect(bodyId).toBe("body_1");
    expect(spec).toMatchObject({
      id: "mine.bracket-plate",
      version: "1.0.0",
      name: "Bracket Plate",
      tags: ["printed", "jig"],
    });
    // One seating attachment, matching the origin convention the dialog states.
    expect(spec.attachments).toEqual({ seat: { on: "face:origin", accepts: ["plane"] } });
    expect(preview).toBe("data:image/png;base64,AAAA");
    expect(onClose).toHaveBeenCalled();
  });

  /*
   * LGU-1 WP-A, defect F10. The dialog opens on the body's own name, which for
   * most bodies is the tree's auto-generated "Body 2" — and `suggestedId` turns
   * that into the permanent `mine.body-2`. A WARNING, never a refusal: the name
   * is legal, and the point is only that the user chose it rather than let it
   * default through.
   */
  it("warns that an auto-generated body name is about to become permanent", async () => {
    render(<SaveAsComponentDialog bodyId="body_1" bodyName="Body 2" onClose={() => {}} />);
    expect(screen.getByTestId("save-as-component-name-warning")).toBeInTheDocument();
    // Still committable — a warning does not disable the action.
    expect(screen.getByTestId("save-as-component-commit")).toBeEnabled();

    const user = userEvent.setup();
    await user.clear(screen.getByTestId("save-as-component-name"));
    await user.type(screen.getByTestId("save-as-component-name"), "Bracket Plate");
    expect(screen.queryByTestId("save-as-component-name-warning")).toBeNull();
  });

  it("does not warn about a name that merely contains a placeholder word", () => {
    render(<SaveAsComponentDialog bodyId="body_1" bodyName="Body Mount" onClose={() => {}} />);
    expect(screen.queryByTestId("save-as-component-name-warning")).toBeNull();
  });

  it("stops following the name once the id is edited by hand", async () => {
    render(<SaveAsComponentDialog bodyId="body_1" bodyName="Plate" onClose={() => {}} />);
    const user = userEvent.setup();

    await user.clear(screen.getByTestId("save-as-component-id"));
    await user.type(screen.getByTestId("save-as-component-id"), "acme.custom");
    await user.type(screen.getByTestId("save-as-component-name"), " Two");

    expect(screen.getByTestId("save-as-component-id")).toHaveValue("acme.custom");
  });

  it("refuses to commit an un-namespaced id without a round trip", async () => {
    render(<SaveAsComponentDialog bodyId="body_1" bodyName="Plate" onClose={() => {}} />);
    const user = userEvent.setup();

    await user.clear(screen.getByTestId("save-as-component-id"));
    await user.type(screen.getByTestId("save-as-component-id"), "plate");

    expect(screen.getByTestId("save-as-component-commit")).toBeDisabled();
    expect(screen.getByTestId("save-as-component-id-error")).toBeInTheDocument();
    expect(saveAsComponent).not.toHaveBeenCalled();
  });

  it("stays open and shows the reason when the backend refuses", async () => {
    saveAsComponent.mockRejectedValue(
      new Error("saveAsComponent: a component must resolve to exactly one solid (spec §9)"),
    );
    const onClose = vi.fn();
    render(<SaveAsComponentDialog bodyId="body_1" bodyName="Plate" onClose={onClose} />);

    await userEvent.setup().click(screen.getByTestId("save-as-component-commit"));

    await waitFor(() =>
      expect(screen.getByTestId("save-as-component-error")).toHaveTextContent("exactly one solid"),
    );
    expect(onClose).not.toHaveBeenCalled();
    expect(screen.getByTestId("save-as-component-name")).toHaveValue("Plate");
  });

  it("renders nothing when no body is being authored", () => {
    render(<SaveAsComponentDialog bodyId={null} onClose={() => {}} />);
    expect(screen.queryByTestId("save-as-component-dialog")).toBeNull();
  });
});

/*
 * Free parameters (WP-F1.3). The whole point is the BINDING: a checked row must
 * travel as `{ key: <variable name>, value: <current> }`, because the re-bake
 * lane sets a variable by name — a parameter that named nothing would be an
 * edit `setComponentParams` could never honour.
 */
describe("SaveAsComponentDialog parameters", () => {
  beforeEach(() => {
    saveAsComponent.mockReset();
    saveAsComponent.mockImplementation((_body: string, spec: NewComponentSpec) =>
      Promise.resolve(saved(spec)),
    );
    listVariables.mockReset();
    listVariables.mockResolvedValue([
      { id: "var_1", name: "depth", value: 10 },
      { id: "var_2", name: "width", value: 20 },
    ]);
    classifyElement.mockReset();
    configureAuthoringController({ geometryQuery: { classifyElement } });
  });

  afterEach(() => {
    cancelAttachmentPick();
    configureAuthoringController(null);
  });

  it("lists the document's variables, one row each", async () => {
    render(<SaveAsComponentDialog bodyId="body_1" bodyName="Plate" onClose={() => {}} />);

    await waitFor(() =>
      expect(screen.getAllByTestId("save-as-component-parameter-row")).toHaveLength(2),
    );
    expect(screen.getByLabelText("Expose depth")).not.toBeChecked();
    expect(screen.queryByTestId("save-as-component-no-variables")).toBeNull();
  });

  it("sends only the checked variables, bound by name to their current value", async () => {
    render(<SaveAsComponentDialog bodyId="body_1" bodyName="Plate" onClose={() => {}} />);
    const user = userEvent.setup();

    await waitFor(() => expect(screen.getByLabelText("Expose depth")).toBeInTheDocument());
    await user.click(screen.getByLabelText("Expose depth"));
    await user.click(screen.getByTestId("save-as-component-commit"));

    await waitFor(() => expect(saveAsComponent).toHaveBeenCalledTimes(1));
    expect(saveAsComponent.mock.calls[0][1].parameters).toEqual({
      depth: { key: "depth", value: 10 },
    });
  });

  it("declares nothing when the author checks nothing — the pre-WP-F1.3 package", async () => {
    render(<SaveAsComponentDialog bodyId="body_1" bodyName="Plate" onClose={() => {}} />);

    await waitFor(() => expect(screen.getByLabelText("Expose depth")).toBeInTheDocument());
    await userEvent.setup().click(screen.getByTestId("save-as-component-commit"));

    await waitFor(() => expect(saveAsComponent).toHaveBeenCalledTimes(1));
    expect(saveAsComponent.mock.calls[0][1].parameters).toEqual({});
  });

  it("says so, rather than showing an empty table, when the document has no variables", async () => {
    listVariables.mockResolvedValue([]);
    render(<SaveAsComponentDialog bodyId="body_1" bodyName="Plate" onClose={() => {}} />);

    await waitFor(() =>
      expect(screen.getByTestId("save-as-component-no-variables")).toBeInTheDocument(),
    );
    expect(screen.queryByTestId("save-as-component-parameters")).toBeNull();
  });
});

/*
 * Attachment picking (WP-F1.2). The interesting property is that a pick becomes
 * a FRAME, not just a label: the placement solver seats a component by its
 * attachment frame, so a row without one would place at the model origin no
 * matter which face the author clicked.
 */
describe("SaveAsComponentDialog attachment picking", () => {
  beforeEach(() => {
    saveAsComponent.mockReset();
    saveAsComponent.mockImplementation((_body: string, spec: NewComponentSpec) =>
      Promise.resolve(saved(spec)),
    );
    listVariables.mockReset();
    listVariables.mockResolvedValue([]);
    classifyElement.mockReset();
    engine.probePick.mockReset();
    engine.setOrbitSuppressed.mockReset();
    configureAuthoringController({ geometryQuery: { classifyElement } });
  });

  afterEach(() => {
    cancelAttachmentPick();
    configureAuthoringController(null);
  });

  it("turns a planar face pick into a seat attachment framed at the clicked point", async () => {
    engine.probePick.mockReturnValue(hitAt([3, 4, 10]));
    classifyElement.mockResolvedValue(PLANE_TOP);
    render(<SaveAsComponentDialog bodyId="body_1" bodyName="Plate" onClose={() => {}} />);
    const user = userEvent.setup();

    await user.click(screen.getByTestId("save-as-component-add-attachment"));
    expect(engine.setOrbitSuppressed).toHaveBeenCalledWith(true);
    await pickInViewport();

    await waitFor(() =>
      expect(screen.getAllByTestId("save-as-component-attachment-row")).toHaveLength(1),
    );
    expect(screen.getByTestId("save-as-component-attachment-name")).toHaveValue("seat");

    await user.click(screen.getByTestId("save-as-component-commit"));
    await waitFor(() => expect(saveAsComponent).toHaveBeenCalledTimes(1));
    const spec = saveAsComponent.mock.calls[0][1] as NewComponentSpec;
    expect(spec.attachments).toEqual({
      seat: {
        on: "face:seat",
        accepts: ["plane"],
        // The clicked point, projected onto the plane — not OCCT's own surface
        // origin, which is wherever the plane happens to be parametrized from.
        frame: { origin: [3, 4, 10], z: [0, 0, 1], x: [1, 0, 0] },
      },
    });
  });

  it("turns a cylinder pick into an axis attachment on the axis line", async () => {
    engine.probePick.mockReturnValue(hitAt([7, 4, 12]));
    classifyElement.mockResolvedValue(CYLINDER);
    render(<SaveAsComponentDialog bodyId="body_1" bodyName="Boss" onClose={() => {}} />);
    const user = userEvent.setup();

    await user.click(screen.getByTestId("save-as-component-add-attachment"));
    await pickInViewport();

    await waitFor(() =>
      expect(screen.getByTestId("save-as-component-attachment-name")).toHaveValue("axis"),
    );
    await user.click(screen.getByTestId("save-as-component-commit"));
    await waitFor(() => expect(saveAsComponent).toHaveBeenCalledTimes(1));
    const spec = saveAsComponent.mock.calls[0][1] as NewComponentSpec;
    expect(spec.attachments.axis.accepts).toEqual(["cylinder", "hole"]);
    expect(spec.attachments.axis.on).toBe("cylinder:axis");
    // The pick projected onto the axis: XY from the axis, Z from the click.
    expect(spec.attachments.axis.frame).toEqual({
      origin: [4, 4, 12],
      z: [0, 0, 1],
      x: [1, 0, 0],
    });
  });

  it("stays in pick mode and says what is pickable when the target has no frame", async () => {
    engine.probePick.mockReturnValue(hitAt([0, 0, 0]));
    classifyElement.mockResolvedValue({
      kind: "face",
      surfaceType: "other",
      curveType: "",
      frame: null,
    } satisfies ClassifyResult);
    render(<SaveAsComponentDialog bodyId="body_1" onClose={() => {}} />);
    const user = userEvent.setup();

    await user.click(screen.getByTestId("save-as-component-add-attachment"));
    await pickInViewport();

    await waitFor(() =>
      expect(screen.getByTestId("save-as-component-pick-hint")).toHaveTextContent(
        /planar face, a cylindrical face, or a circular edge/,
      ),
    );
    expect(screen.queryByTestId("save-as-component-attachment-row")).toBeNull();
    // Still armed — the author corrects their aim rather than re-arming.
    expect(screen.getByTestId("save-as-component-add-attachment")).toHaveTextContent(
      "Stop picking",
    );
  });

  it("names a second attachment of the same kind apart, and refuses a duplicate rename", async () => {
    engine.probePick.mockReturnValue(hitAt([0, 0, 10]));
    classifyElement.mockResolvedValue(PLANE_TOP);
    render(<SaveAsComponentDialog bodyId="body_1" onClose={() => {}} />);
    const user = userEvent.setup();

    await user.click(screen.getByTestId("save-as-component-add-attachment"));
    await pickInViewport();
    await waitFor(() =>
      expect(screen.getAllByTestId("save-as-component-attachment-row")).toHaveLength(1),
    );
    classifyElement.mockClear();
    await pickInViewport();
    await waitFor(() =>
      expect(screen.getAllByTestId("save-as-component-attachment-row")).toHaveLength(2),
    );
    const names = screen.getAllByTestId("save-as-component-attachment-name");
    expect((names[1] as HTMLInputElement).value).toBe("seat2");

    // Renaming the second onto the first is refused before a round trip: a
    // duplicate key would silently drop one attachment from the manifest.
    await user.clear(names[1]);
    await user.type(names[1], "seat");
    expect(screen.getByTestId("save-as-component-attachment-error")).toHaveTextContent("unique");
    expect(screen.getByTestId("save-as-component-commit")).toBeDisabled();

    // An out-of-charset name is refused too — this is a manifest table key.
    await user.clear(names[1]);
    await user.type(names[1], "Seat Two");
    expect(screen.getByTestId("save-as-component-attachment-error")).toHaveTextContent(
      "lowercase",
    );
  });

  it("removes a row, and drops back to the model-origin seat when none are left", async () => {
    engine.probePick.mockReturnValue(hitAt([0, 0, 10]));
    classifyElement.mockResolvedValue(PLANE_TOP);
    render(<SaveAsComponentDialog bodyId="body_1" bodyName="Plate" onClose={() => {}} />);
    const user = userEvent.setup();

    await user.click(screen.getByTestId("save-as-component-add-attachment"));
    await pickInViewport();
    await waitFor(() =>
      expect(screen.getAllByTestId("save-as-component-attachment-row")).toHaveLength(1),
    );
    await user.click(screen.getByTestId("save-as-component-attachment-remove"));
    await user.click(screen.getByTestId("save-as-component-commit"));

    await waitFor(() => expect(saveAsComponent).toHaveBeenCalledTimes(1));
    const spec = saveAsComponent.mock.calls[0][1] as NewComponentSpec;
    expect(spec.attachments).toEqual({ seat: { on: "face:origin", accepts: ["plane"] } });
  });

  it("leaves pick mode on Esc WITHOUT closing the dialog", async () => {
    engine.probePick.mockReturnValue(hitAt([0, 0, 10]));
    classifyElement.mockResolvedValue(PLANE_TOP);
    const onClose = vi.fn();
    render(<SaveAsComponentDialog bodyId="body_1" bodyName="Plate" onClose={onClose} />);
    const user = userEvent.setup();

    await user.click(screen.getByTestId("save-as-component-add-attachment"));
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));

    await waitFor(() =>
      expect(screen.getByTestId("save-as-component-add-attachment")).toHaveTextContent(
        "Add attachment",
      ),
    );
    expect(onClose).not.toHaveBeenCalled();
    expect(screen.getByTestId("save-as-component-dialog")).toBeInTheDocument();
    expect(engine.setOrbitSuppressed).toHaveBeenLastCalledWith(false);
    expect(screen.getByTestId("save-as-component-name")).toHaveValue("Plate");

    // Disarmed: a viewport click is nobody's business now.
    window.dispatchEvent(new MouseEvent("pointerdown", { clientX: 40, clientY: 40 }));
    expect(classifyElement).not.toHaveBeenCalled();
  });
});

/*
 * The multi-solid refusal (WP-F1.2). The dialog keys its offer on the MARKER in
 * the message, so a reworded refusal cannot silently turn the offer off.
 */
describe("SaveAsComponentDialog union-at-bake offer", () => {
  beforeEach(() => {
    saveAsComponent.mockReset();
    configureAuthoringController({ geometryQuery: { classifyElement } });
  });

  afterEach(() => {
    cancelAttachmentPick();
    configureAuthoringController(null);
  });

  it("offers the fuse on a multi-solid refusal and re-submits with unionSolids", async () => {
    saveAsComponent.mockRejectedValueOnce(
      new Error(
        "invalid command: saveAsComponent: MULTI_SOLID_BODY: a component must resolve to exactly one solid (spec §9); body_1 baked 2",
      ),
    );
    const onClose = vi.fn();
    render(<SaveAsComponentDialog bodyId="body_1" bodyName="Pair" onClose={onClose} />);
    const user = userEvent.setup();

    await user.click(screen.getByTestId("save-as-component-commit"));
    await waitFor(() =>
      expect(screen.getByTestId("save-as-component-union-offer")).toBeInTheDocument(),
    );
    expect(onClose).not.toHaveBeenCalled();

    saveAsComponent.mockImplementation((_body: string, spec: NewComponentSpec) =>
      Promise.resolve(saved(spec)),
    );
    await user.click(screen.getByTestId("save-as-component-union-commit"));

    await waitFor(() => expect(saveAsComponent).toHaveBeenCalledTimes(2));
    expect(saveAsComponent.mock.calls[0][3]).toBe(false);
    expect(saveAsComponent.mock.calls[1][3]).toBe(true);
    await waitFor(() => expect(onClose).toHaveBeenCalled());
  });

  it("does not offer the fuse for any other refusal", async () => {
    saveAsComponent.mockRejectedValue(
      new Error("invalid command: saveAsComponent: mine.pair@1.0.0 already exists"),
    );
    render(<SaveAsComponentDialog bodyId="body_1" bodyName="Pair" onClose={() => {}} />);

    await userEvent.setup().click(screen.getByTestId("save-as-component-commit"));

    await waitFor(() =>
      expect(screen.getByTestId("save-as-component-error")).toHaveTextContent("already exists"),
    );
    expect(screen.queryByTestId("save-as-component-union-offer")).toBeNull();
  });

  it("reports a refused union plainly instead of offering it again", async () => {
    saveAsComponent.mockRejectedValueOnce(
      new Error("saveAsComponent: MULTI_SOLID_BODY: … baked 2"),
    );
    render(<SaveAsComponentDialog bodyId="body_1" bodyName="Pair" onClose={() => {}} />);
    const user = userEvent.setup();

    await user.click(screen.getByTestId("save-as-component-commit"));
    await waitFor(() =>
      expect(screen.getByTestId("save-as-component-union-offer")).toBeInTheDocument(),
    );

    // The worker refuses a fuse of disjoint bodies — with the SAME marker,
    // because the bake still produced more than one solid.
    saveAsComponent.mockRejectedValue(
      new Error("saveAsComponent: MULTI_SOLID_BODY: union refused — fused result carries 2 solids"),
    );
    await user.click(screen.getByTestId("save-as-component-union-commit"));

    await waitFor(() =>
      expect(screen.getByTestId("save-as-component-error")).toHaveTextContent("union refused"),
    );
    expect(screen.queryByTestId("save-as-component-union-offer")).toBeNull();
  });
});
