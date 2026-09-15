import { afterEach, describe, expect, it, vi } from "vitest";
import { useRef, useState } from "react";
import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Popover } from "@/ui/Popover";

function Harness({
  autoFocus = "none",
  childAutoFocus = false,
  width = 238,
}: {
  autoFocus?: "first" | "none";
  childAutoFocus?: boolean;
  width?: number;
}) {
  const anchor = useRef<HTMLButtonElement | null>(null);
  const [open, setOpen] = useState(true);
  return (
    <div>
      <button ref={anchor} type="button" onClick={() => setOpen((v) => !v)}>
        anchor
      </button>
      <button type="button">outside</button>
      <Popover
        open={open}
        onClose={() => setOpen(false)}
        anchorRef={anchor}
        caret
        ariaLabel="Test popover"
        autoFocus={autoFocus}
        width={width}
      >
        <div>
          <button type="button">first control</button>
          <input aria-label="draft" autoFocus={childAutoFocus} />
          <div>popover-body</div>
        </div>
      </Popover>
    </div>
  );
}

const initialViewport = { height: window.innerHeight, width: window.innerWidth };

afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  Object.defineProperties(window, {
    innerHeight: { configurable: true, value: initialViewport.height },
    innerWidth: { configurable: true, value: initialViewport.width },
  });
});

describe("Popover", () => {
  it("renders its children when open", () => {
    render(<Harness />);
    expect(screen.getByText("popover-body")).toBeInTheDocument();
    expect(screen.getByRole("dialog", { name: "Test popover" })).toBeInTheDocument();
  });

  it("moves opt-in keyboard focus inside and preserves normal Tab order", async () => {
    const user = userEvent.setup();
    render(<Harness autoFocus="first" />);
    await waitFor(() => expect(screen.getByRole("button", { name: "first control" })).toHaveFocus());
    await user.tab();
    expect(screen.getByRole("textbox", { name: "draft" })).toHaveFocus();
  });

  it("preserves a child autofocus target ahead of the first DOM control", async () => {
    render(<Harness autoFocus="first" childAutoFocus />);
    await waitFor(() => expect(screen.getByRole("textbox", { name: "draft" })).toHaveFocus());
  });

  it("closes on Escape and returns focus to its anchor", async () => {
    const user = userEvent.setup();
    render(<Harness autoFocus="first" />);
    await user.keyboard("{Escape}");
    expect(screen.queryByText("popover-body")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "anchor" })).toHaveFocus();
  });

  it("retains legacy Escape closing while focus stays on an existing trigger", async () => {
    const user = userEvent.setup();
    render(<Harness />);
    screen.getByRole("button", { name: "anchor" }).focus();
    await user.keyboard("{Escape}");
    expect(screen.queryByText("popover-body")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "anchor" })).toHaveFocus();
  });

  it("marks the open trigger before capture listeners inspect Escape", async () => {
    const user = userEvent.setup();
    const cancelCad = vi.fn();
    const capture = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      const inPopupScope = event.composedPath().some((node) => (
        node instanceof Element && node.matches("[data-cad-keyboard-scope]")
      ));
      if (!inPopupScope) cancelCad();
    };
    window.addEventListener("keydown", capture, true);
    try {
      render(<Harness />);
      const anchor = screen.getByRole("button", { name: "anchor" });
      expect(anchor).toHaveAttribute("data-cad-keyboard-scope");
      anchor.focus();
      await user.keyboard("{Escape}");
      expect(cancelCad).not.toHaveBeenCalled();
      expect(screen.queryByText("popover-body")).not.toBeInTheDocument();
      expect(anchor).not.toHaveAttribute("data-cad-keyboard-scope");
    } finally {
      window.removeEventListener("keydown", capture, true);
    }
  });

  it("closes on outside pointer without stealing focus from the clicked target", async () => {
    const user = userEvent.setup();
    render(<Harness autoFocus="first" />);
    expect(screen.getByText("popover-body")).toBeInTheDocument();
    await user.click(screen.getByText("outside"));
    expect(screen.queryByText("popover-body")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "outside" })).toHaveFocus();
  });

  it("keeps open when clicking inside the panel", async () => {
    const user = userEvent.setup();
    render(<Harness />);
    await user.click(screen.getByText("popover-body"));
    expect(screen.getByText("popover-body")).toBeInTheDocument();
  });

  it("cleans its ResizeObserver when closed", async () => {
    const disconnect = vi.fn();
    class ResizeObserverMock {
      observe = vi.fn();
      disconnect = disconnect;
    }
    vi.stubGlobal("ResizeObserver", ResizeObserverMock);
    const user = userEvent.setup();
    render(<Harness />);
    await user.click(screen.getByRole("button", { name: "anchor" }));
    expect(disconnect).toHaveBeenCalledOnce();
  });

  it("clamps width and position to a narrow viewport, then updates on resize", async () => {
    Object.defineProperties(window, {
      innerHeight: { configurable: true, value: 180 },
      innerWidth: { configurable: true, value: 180 },
    });
    render(<Harness width={238} />);
    const anchor = screen.getByRole("button", { name: "anchor" });
    vi.spyOn(anchor, "getBoundingClientRect").mockReturnValue({
      bottom: 170,
      height: 20,
      left: 170,
      right: 190,
      top: 150,
      width: 20,
      x: 170,
      y: 150,
      toJSON: () => ({}),
    });
    act(() => window.dispatchEvent(new Event("resize")));
    const dialog = screen.getByRole("dialog");
    await waitFor(() => {
      // maxHeight clears BOTTOM_INSET (34px status bar) in addition to the
      // viewport margin: 180 - 34 - 8*2 = 130.
      expect(dialog).toHaveStyle({ left: "8px", width: "164px", maxHeight: "130px" });
    });

    Object.defineProperty(window, "innerWidth", { configurable: true, value: 300 });
    act(() => window.dispatchEvent(new Event("resize")));
    await waitFor(() => expect(dialog).toHaveStyle({ left: "54px", width: "238px" }));
  });

  // C2: a tall popover must stay clear of the status bar (BOTTOM_INSET) rather
  // than clipping under it, and must scroll instead of overflowing silently.
  it("C2: clamps maxHeight above the status bar and keeps overflowY scrollable", async () => {
    Object.defineProperties(window, {
      innerHeight: { configurable: true, value: 400 },
      innerWidth: { configurable: true, value: 800 },
    });
    render(<Harness />);
    const anchor = screen.getByRole("button", { name: "anchor" });
    vi.spyOn(anchor, "getBoundingClientRect").mockReturnValue({
      bottom: 380,
      height: 20,
      left: 400,
      right: 420,
      top: 360,
      width: 20,
      x: 400,
      y: 360,
      toJSON: () => ({}),
    });
    act(() => window.dispatchEvent(new Event("resize")));
    const dialog = screen.getByRole("dialog");
    await waitFor(() => {
      // 400 - 34 (status bar) - 8*2 (viewport margin) = 350 — NOT the naive
      // 400 - 16 = 384 a status-bar-blind clamp would produce.
      expect(dialog).toHaveStyle({ maxHeight: "350px", overflowY: "auto" });
    });
  });
});
