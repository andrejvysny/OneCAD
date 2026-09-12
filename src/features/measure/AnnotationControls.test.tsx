import { act, fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";
import { measurementAnnotationStore } from "@/stores/measurementAnnotationStore";
import { AnnotationControls } from "./AnnotationControls";

describe("AnnotationControls", () => {
  beforeEach(() => measurementAnnotationStore.getState().reset());

  it("keeps per-label hide and pin controls accessible in the docked reading", () => {
    act(() => {
      measurementAnnotationStore.getState().reconcile({ a: "face:body1:a", pair: "face:body1:a|face:body1:b" });
      measurementAnnotationStore.getState().setLivePosition("a", { x: 24, y: 40 });
      measurementAnnotationStore.getState().setPlacement("a", "visible");
      measurementAnnotationStore.getState().setPlacement("pair", "no-space");
    });
    render(<AnnotationControls />);

    expect(screen.getByTestId("measurement-annotation-controls")).toHaveAttribute("data-viewport-interactive");
    fireEvent.click(screen.getByText("Annotation labels"));
    expect(screen.getByRole("button", { name: "Hide A label" })).toBeEnabled();
    expect(screen.getByRole("button", { name: "Pin A label" })).toBeEnabled();
    expect(screen.getByRole("status")).toHaveTextContent("Hidden: no clear space");
    expect(screen.getByRole("button", { name: "Pin Pair label" })).toBeDisabled();

    fireEvent.click(screen.getByRole("button", { name: "Pin A label" }));
    expect(screen.getByRole("button", { name: "Unpin A label" })).toBeEnabled();
    fireEvent.click(screen.getByRole("button", { name: "Hide A label" }));
    expect(screen.getByRole("button", { name: "Show A label" })).toBeEnabled();
    expect(screen.getByRole("button", { name: "Unpin A label" })).toBeEnabled();
  });
});
