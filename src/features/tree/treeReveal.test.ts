import { describe, expect, it, vi } from "vitest";
import { requestTreeReveal, subscribeTreeReveal } from "./treeReveal";

const target = {
  providerId: "onecad.demo.tree.provider" as never,
  nodeId: "item-1",
};

describe("tree reveal channel", () => {
  it("does not queue a request while no tree host is mounted", () => {
    expect(requestTreeReveal(target)).toBe(false);
  });

  it("notifies mounted hosts and stops after their cleanup", () => {
    const listener = vi.fn(() => true);
    const unsubscribe = subscribeTreeReveal(listener);

    expect(requestTreeReveal(target)).toBe(true);
    expect(listener).toHaveBeenCalledWith(target);

    unsubscribe();
    expect(requestTreeReveal(target)).toBe(false);
  });
});
