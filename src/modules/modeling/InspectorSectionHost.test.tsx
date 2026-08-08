/*
 * The host's own contract, separate from the section-order golden probe.
 *
 * `canRender` is a plain predicate — nothing re-runs it on its own — so the
 * interesting failures are all STALENESS: a selection or mode change that does
 * not re-evaluate admission, and a registry change that does not re-render.
 * Each case here would pass against a host that subscribed to the registry only.
 */
import { describe, it, expect, beforeEach } from "vitest";
import { screen, act } from "@testing-library/react";
import { contributionId, type InspectorContributionId, type ModuleScope } from "@/platform";
import { renderWithPlatform } from "@/test/renderWithPlatform";
import { resetStores } from "@/test/resetStores";
import { selectionStore } from "@/stores/selectionStore";
import { toolStore } from "@/stores/toolStore";
import { InspectorSectionHost } from "./InspectorSectionHost";
import { MODELING_MODULE_ID, ModelingScopes } from "./manifest";

const id = (name: string) =>
  contributionId<InspectorContributionId>(MODELING_MODULE_ID, `onecad.modeling.inspector.${name}`);

const Body = () => <div>BODY</div>;
const Sketching = () => <div>SKETCHING</div>;
const Late = () => <div>LATE</div>;
const Boom = () => {
  throw new Error("section blew up");
};

function contributeProbes(scope: ModuleScope): void {
  scope.registerInspectorSection({
    id: id("probe.body"),
    priority: 100,
    canRender: (ctx) => ctx.selection[0]?.typeId === "body",
    component: Body,
  });
  scope.registerInspectorSection({
    id: id("probe.sketching"),
    priority: 200,
    canRender: (ctx) => ctx.scopes.includes(ModelingScopes.Sketch),
    component: Sketching,
  });
}

describe("InspectorSectionHost", () => {
  beforeEach(() => resetStores());

  it("admits a section when the SELECTION starts matching, and drops it again", () => {
    renderWithPlatform(<InspectorSectionHost />, { contribute: contributeProbes });
    expect(screen.queryByText("BODY")).toBeNull();

    act(() => selectionStore.getState().set([{ kind: "body", id: "body1" }]));
    expect(screen.getByText("BODY")).toBeInTheDocument();

    act(() => selectionStore.getState().clear());
    expect(screen.queryByText("BODY")).toBeNull();
  });

  it("re-evaluates admission when the MODE changes", () => {
    renderWithPlatform(<InspectorSectionHost />, { contribute: contributeProbes });
    expect(screen.queryByText("SKETCHING")).toBeNull();

    act(() => toolStore.getState().setMode("sketch", "sketch2"));
    expect(screen.getByText("SKETCHING")).toBeInTheDocument();

    act(() => toolStore.getState().setMode("model"));
    expect(screen.queryByText("SKETCHING")).toBeNull();
  });

  it("picks up a section registered AFTER mount, and loses it on dispose", () => {
    const { platform } = renderWithPlatform(<InspectorSectionHost />, {
      contribute: contributeProbes,
    });
    expect(screen.queryByText("LATE")).toBeNull();

    const scope = platform.createScope(MODELING_MODULE_ID);
    act(() => {
      scope.registerInspectorSection({
        id: id("probe.late"),
        priority: 50,
        canRender: () => true,
        component: Late,
      });
    });
    expect(screen.getByText("LATE")).toBeInTheDocument();

    act(() => scope.dispose());
    expect(screen.queryByText("LATE")).toBeNull();
  });

  it("orders by priority, never by registration order", () => {
    const { container } = renderWithPlatform(<InspectorSectionHost />, {
      contribute: (scope) => {
        // Registered high-priority-first on purpose: insertion order is the
        // tie-break only, and must not decide this.
        scope.registerInspectorSection({
          id: id("probe.second"),
          priority: 200,
          canRender: () => true,
          component: () => <div>SECOND</div>,
        });
        scope.registerInspectorSection({
          id: id("probe.first"),
          priority: 100,
          canRender: () => true,
          component: () => <div>FIRST</div>,
        });
      },
    });
    expect(container.textContent).toBe("FIRSTSECOND");
  });

  it("isolates a throwing section — the rest of the panel survives", () => {
    renderWithPlatform(<InspectorSectionHost />, {
      contribute: (scope) => {
        scope.registerInspectorSection({
          id: id("probe.boom"),
          priority: 100,
          canRender: () => true,
          component: Boom,
        });
        scope.registerInspectorSection({
          id: id("probe.late"),
          priority: 200,
          canRender: () => true,
          component: Late,
        });
      },
    });
    expect(screen.getByTestId("contribution-error")).toBeInTheDocument();
    expect(screen.getByText("LATE")).toBeInTheDocument();
  });

  it("hands the platform a SelectionRef, not a modeling EntityRef", () => {
    let seen: readonly { typeId: string; subElement?: string }[] = [];
    renderWithPlatform(<InspectorSectionHost />, {
      contribute: (scope) =>
        scope.registerInspectorSection({
          id: id("probe.spy"),
          canRender: (ctx) => {
            seen = ctx.selection;
            return false;
          },
          component: Late,
        }),
    });
    act(() =>
      selectionStore.getState().set([
        { kind: "face", id: "body1#f:0", bodyId: "body1", topoKey: "f:0", elementId: "el_top" },
      ]),
    );
    expect(seen).toEqual([
      { entityId: "body1#f:0", owner: MODELING_MODULE_ID, typeId: "face", subElement: "el_top" },
    ]);
  });

  it("does NOT pass a topoKey off as an ElementId", () => {
    // A face with no promoted id must not look promoted: `subElement` is the
    // evidence Appearance gates on, and a topoKey is snapshot-scoped.
    let seen: readonly { subElement?: string }[] = [];
    renderWithPlatform(<InspectorSectionHost />, {
      contribute: (scope) =>
        scope.registerInspectorSection({
          id: id("probe.spy"),
          canRender: (ctx) => {
            seen = ctx.selection;
            return false;
          },
          component: Late,
        }),
    });
    act(() =>
      selectionStore
        .getState()
        .set([{ kind: "face", id: "body1#f:1", bodyId: "body1", topoKey: "f:1" }]),
    );
    expect(seen[0]?.subElement).toBeUndefined();
  });
  it("a throwing canRender does not take the panel down with it", () => {
    // canRender runs BEFORE the section has a ContributionBoundary — the boundary
    // only catches what its children throw — so an unguarded predicate escapes
    // past the section and kills the whole inspector.
    renderWithPlatform(<InspectorSectionHost />, {
      contribute: (scope) => {
        scope.registerInspectorSection({
          id: id("probe.badPredicate"),
          priority: 100,
          canRender: () => {
            throw new Error("predicate blew up");
          },
          component: Late,
        });
        scope.registerInspectorSection({
          id: id("probe.healthy"),
          priority: 200,
          canRender: () => true,
          component: Late,
        });
      },
    });
    expect(screen.getByText("LATE")).toBeInTheDocument();
  });
});
