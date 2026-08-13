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
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import {
  isValidComponentId,
  isValidVersion,
  suggestedId,
  SaveAsComponentDialog,
} from "./SaveAsComponentDialog";
import type { LibraryComponent, NewComponentSpec } from "@/ipc/types";

const saveAsComponent = vi.fn();

vi.mock("@/ipc/client", () => ({
  createClient: () => ({ saveAsComponent: (...args: unknown[]) => saveAsComponent(...args) }),
}));
vi.mock("@/viewport/engineBridge", () => ({
  getViewportEngine: () => ({ captureThumbnail: () => "data:image/png;base64,AAAA" }),
}));

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
