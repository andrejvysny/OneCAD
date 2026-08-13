/*
 * The start screen's Templates row (spec §8; WP-B3) — the nav key that has
 * been a stub since the start screen shipped.
 *
 * The claim worth pinning is the one a user can be hurt by: choosing a
 * template starts a NEW document from it and never opens the template itself.
 */
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ProjectTemplate } from "@/ipc/types";
import { TemplateGrid } from "./TemplateGrid";

const TEMPLATES: ProjectTemplate[] = [
  { id: "blank", name: "Blank", description: "An empty document" },
  { id: "printer-part", name: "3D-printer part", previewDataUrl: "data:image/png;base64,AAAA" },
];

describe("TemplateGrid", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
  });

  it("renders one card per template, with its own preview when it has one", () => {
    const { container } = render(<TemplateGrid templates={TEMPLATES} onUse={() => {}} />);
    expect(screen.getAllByTestId("template-card")).toHaveLength(2);
    expect(screen.getByText("Blank")).toBeInTheDocument();
    // `alt=""` on purpose — the thumbnail carries no information the card's
    // own name does not, so it is decorative and has no img role to query.
    expect(container.querySelector("img")).toHaveAttribute(
      "src",
      "data:image/png;base64,AAAA",
    );
  });

  it("falls back to a description line that says what choosing one does", () => {
    render(<TemplateGrid templates={[TEMPLATES[1]]} onUse={() => {}} />);
    expect(screen.getByText("Starts a new untitled project")).toBeInTheDocument();
  });

  it("hands the template ID to `onUse` — never a path to open", async () => {
    const onUse = vi.fn();
    render(<TemplateGrid templates={TEMPLATES} onUse={onUse} />);
    await userEvent.setup().click(screen.getByText("Blank"));
    await waitFor(() => expect(onUse).toHaveBeenCalledWith("blank"));
  });

  it("renders nothing for an empty list rather than an empty grid frame", () => {
    render(<TemplateGrid templates={[]} onUse={() => {}} />);
    expect(screen.queryByTestId("template-card")).toBeNull();
  });
});
