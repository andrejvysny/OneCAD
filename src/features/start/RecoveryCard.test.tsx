import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { RecoveryCard } from "./RecoveryCard";
import type { RecoveryInfo } from "@/ipc/types";

function offer(over: Partial<RecoveryInfo> = {}): RecoveryInfo {
  return {
    documentId: "11111111-1111-1111-1111-111111111111",
    title: "Bracket",
    originalPath: "/docs/Bracket.onecad",
    autosavePath: "/x/autosave/foo.onecad",
    modifiedMs: Date.UTC(2026, 7, 15, 14, 32),
    ...over,
  };
}

const noop = () => {};

describe("RecoveryCard — naming an offer", () => {
  it("prefers the recorded title", () => {
    render(<RecoveryCard recovery={[offer()]} pendingId={null} onRestore={noop} onDiscard={noop} />);
    expect(screen.getByText("Bracket")).toBeInTheDocument();
  });

  it("falls back to the saved file's stem when the marker recorded no title", () => {
    render(
      <RecoveryCard
        recovery={[offer({ title: undefined })]}
        pendingId={null}
        onRestore={noop}
        onDiscard={noop}
      />,
    );
    expect(screen.getByText("Bracket")).toBeInTheDocument();
  });

  /**
   * An ORPHAN — a container whose session marker did not survive — has neither a
   * title nor a path. The label must stay generic: the autosave file is named
   * `<documentId>.onecad`, so anything derived from it would show a raw UUID.
   */
  it("labels an orphan generically, never with the document id", () => {
    render(
      <RecoveryCard
        recovery={[offer({ title: undefined, originalPath: undefined })]}
        pendingId={null}
        onRestore={noop}
        onDiscard={noop}
      />,
    );
    expect(screen.getByText("Untitled document")).toBeInTheDocument();
    expect(screen.queryByText(/1111-1111/)).not.toBeInTheDocument();
  });
});

describe("RecoveryCard — the timestamp", () => {
  /**
   * "How much work is in this?" is the only question the user has, and a date alone
   * renders a crash ninety seconds ago identically to one last Tuesday.
   */
  it("shows the time of day, not just the date", () => {
    render(<RecoveryCard recovery={[offer()]} pendingId={null} onRestore={noop} onDiscard={noop} />);
    const line = screen.getByText(/autosaved/);
    expect(line.textContent).toMatch(/Aug 15, 2026/);
    expect(line.textContent).toMatch(/\d{1,2}:\d{2}/);
  });

  /** `modifiedMs: 0` means the filesystem could not answer — not 1 Jan 1970. */
  it("shows no timestamp at all when the mtime is unavailable", () => {
    render(
      <RecoveryCard
        recovery={[offer({ modifiedMs: 0 })]}
        pendingId={null}
        onRestore={noop}
        onDiscard={noop}
      />,
    );
    expect(screen.queryByText(/autosaved/)).not.toBeInTheDocument();
    expect(screen.queryByText(/1970/)).not.toBeInTheDocument();
  });
});

describe("RecoveryCard — several crashed sessions", () => {
  it("renders every offer, each independently actionable", async () => {
    const user = userEvent.setup();
    const onRestore = vi.fn();
    const a = offer({ documentId: "a", title: "Bracket" });
    const b = offer({ documentId: "b", title: "Housing" });
    render(
      <RecoveryCard recovery={[a, b]} pendingId={null} onRestore={onRestore} onDiscard={noop} />,
    );

    expect(screen.getByText("2 documents with unsaved changes")).toBeInTheDocument();
    expect(screen.getByText("Bracket")).toBeInTheDocument();
    expect(screen.getByText("Housing")).toBeInTheDocument();

    await user.click(screen.getAllByRole("button", { name: /Restore/ })[1]);
    expect(onRestore).toHaveBeenCalledExactlyOnceWith("b");
  });

  it("renders nothing when there is nothing to recover", () => {
    const { container } = render(
      <RecoveryCard recovery={[]} pendingId={null} onRestore={noop} onDiscard={noop} />,
    );
    expect(container).toBeEmptyDOMElement();
  });
});

describe("RecoveryCard — a decision in flight", () => {
  /**
   * A restore can take seconds (the recovered document regens from op 0). Without a
   * disabled state a second click fired a second call, which found nothing pending
   * and silently did nothing — indistinguishable from the app hanging.
   */
  it("disables every button while a decision is pending", () => {
    render(
      <RecoveryCard
        recovery={[offer({ documentId: "a" }), offer({ documentId: "b" })]}
        pendingId="a"
        onRestore={noop}
        onDiscard={noop}
      />,
    );
    for (const button of screen.getAllByRole("button")) {
      expect(button).toBeDisabled();
    }
    expect(screen.getByRole("button", { name: "Restoring…" })).toBeInTheDocument();
  });
});
