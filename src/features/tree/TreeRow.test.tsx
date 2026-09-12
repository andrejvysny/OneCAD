import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { TreeRow } from "./TreeRow";

function row(startHover?: () => { dispose(): void }) {
  return (
    <TreeRow
      name="Transient row"
      icon="cube"
      selected={false}
      onSelect={() => {}}
      onHoverStart={startHover}
    />
  );
}

describe("TreeRow hover lease", () => {
  it("settles the original lease after a provider re-projection", () => {
    const first = { dispose: vi.fn() };
    const second = { dispose: vi.fn() };
    const { rerender } = render(row(() => first));
    const option = screen.getByRole("option", { name: "Transient row" });

    fireEvent.pointerEnter(option);
    rerender(row(() => second));
    fireEvent.pointerLeave(option);

    expect(first.dispose).toHaveBeenCalledOnce();
    expect(second.dispose).not.toHaveBeenCalled();
  });

  it("settles a live lease when the row unmounts", () => {
    const lease = { dispose: vi.fn() };
    const { unmount } = render(row(() => lease));

    fireEvent.pointerEnter(screen.getByRole("option", { name: "Transient row" }));
    unmount();

    expect(lease.dispose).toHaveBeenCalledOnce();
  });
});
