/*
 * SlotHost: deterministic render order, live re-render on registration change,
 * and per-contribution failure isolation (ADR-0003, ADR-0007, spec §129).
 */
import { describe, it, expect, vi } from "vitest";
import { render, screen, act } from "@testing-library/react";
import { createPlatform, type Platform } from "../platform";
import { contributionId, moduleId, type PanelId } from "../ids";
import { Slots } from "../slots";
import { PlatformProvider } from "./PlatformProvider";
import { SlotHost } from "./SlotHost";

const A = moduleId("onecad.a");
const panelId = (name: string) => contributionId<PanelId>(A, `onecad.a.${name}`);

const label = (text: string) => () => <div>{text}</div>;

function mount(platform: Platform, slot = Slots.ShellLeft) {
  return render(
    <PlatformProvider platform={platform}>
      <SlotHost slot={slot} />
    </PlatformProvider>,
  );
}

describe("SlotHost", () => {
  it("renders only the contributions placed in its slot", () => {
    const platform = createPlatform();
    const scope = platform.scopeFor(A);
    scope.registerPanel({ id: panelId("left"), slot: Slots.ShellLeft, component: label("LEFT") });
    scope.registerPanel({ id: panelId("right"), slot: Slots.ShellRight, component: label("RIGHT") });

    mount(platform);
    expect(screen.getByText("LEFT")).toBeInTheDocument();
    expect(screen.queryByText("RIGHT")).toBeNull();
  });

  it("renders in priority order, not registration order", () => {
    const platform = createPlatform();
    const scope = platform.scopeFor(A);
    scope.registerPanel({
      id: panelId("last"),
      slot: Slots.ShellLeft,
      priority: 300,
      component: label("C"),
    });
    scope.registerPanel({
      id: panelId("first"),
      slot: Slots.ShellLeft,
      priority: 100,
      component: label("A"),
    });
    scope.registerPanel({
      id: panelId("middle"),
      slot: Slots.ShellLeft,
      priority: 200,
      component: label("B"),
    });

    const { container } = mount(platform);
    expect(container.textContent).toBe("ABC");
  });

  it("re-renders when a contribution is added or removed", () => {
    const platform = createPlatform();
    const scope = platform.scopeFor(A);
    const { container } = mount(platform);
    expect(container.textContent).toBe("");

    act(() =>
      void scope.registerPanel({
        id: panelId("late"),
        slot: Slots.ShellLeft,
        component: label("LATE"),
      }),
    );
    expect(container.textContent).toBe("LATE");

    act(() => scope.dispose());
    expect(container.textContent).toBe("");
  });

  it("a throwing contribution does not take down its neighbours", () => {
    // React logs the caught error; silence it so the suite output stays readable.
    const consoleError = vi.spyOn(console, "error").mockImplementation(() => {});
    const platform = createPlatform();
    const scope = platform.scopeFor(A);
    const Broken = () => {
      throw new Error("contribution exploded");
    };
    scope.registerPanel({
      id: panelId("broken"),
      slot: Slots.ShellLeft,
      priority: 100,
      component: Broken,
    });
    scope.registerPanel({
      id: panelId("healthy"),
      slot: Slots.ShellLeft,
      priority: 200,
      component: label("HEALTHY"),
    });

    mount(platform);

    expect(screen.getByText("HEALTHY")).toBeInTheDocument();
    expect(screen.getByTestId("contribution-error")).toHaveAttribute(
      "data-contribution-error",
      "onecad.a.broken",
    );
    consoleError.mockRestore();
  });

  it("throws a directed message when used outside a provider", () => {
    const consoleError = vi.spyOn(console, "error").mockImplementation(() => {});
    expect(() => render(<SlotHost slot={Slots.ShellLeft} />)).toThrowError(/PlatformProvider/);
    consoleError.mockRestore();
  });
});
