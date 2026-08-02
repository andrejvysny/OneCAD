import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { ErrorBoundary } from "./ErrorBoundary";
import { __resetLogForTests, logSnapshot } from "@/debug/log";

function Boom({ throws }: { throws: boolean }) {
  if (throws) throw new Error("render exploded");
  return <p>recovered</p>;
}

describe("ErrorBoundary", () => {
  // React re-throws to console.error for every caught boundary error; that is
  // React's own logging, not ours, and it would drown the runner output.
  let reactNoise: ReturnType<typeof vi.spyOn>;

  beforeEach(() => {
    __resetLogForTests();
    reactNoise = vi.spyOn(console, "error").mockImplementation(() => {});
  });

  afterEach(() => {
    reactNoise.mockRestore();
    __resetLogForTests({ enabled: false });
  });

  it("renders its children untouched when nothing throws", () => {
    render(
      <ErrorBoundary>
        <p>all good</p>
      </ErrorBoundary>,
    );
    expect(screen.getByText("all good")).toBeInTheDocument();
    expect(logSnapshot()).toEqual([]);
  });

  it("catches a render throw, shows the fallback, and logs it with the componentStack", () => {
    render(
      <ErrorBoundary>
        <Boom throws />
      </ErrorBoundary>,
    );

    expect(screen.getByTestId("error-boundary")).toBeInTheDocument();
    expect(screen.getByRole("alert")).toHaveTextContent("render exploded");

    const events = logSnapshot();
    expect(events).toHaveLength(1);
    expect(events[0]).toMatchObject({ level: "error", tag: "err" });
    expect(events[0].msg).toContain("render exploded");
    const ctx = events[0].ctx as { componentStack?: string; name?: string };
    expect(ctx.name).toBe("Error");
    expect(ctx.componentStack).toContain("Boom");
  });

  it("'Try again' clears the error so a transient failure needs no app restart", async () => {
    const user = userEvent.setup();
    const { rerender } = render(
      <ErrorBoundary>
        <Boom throws />
      </ErrorBoundary>,
    );
    expect(screen.getByTestId("error-boundary")).toBeInTheDocument();

    // Swap in a child that no longer throws, then retry.
    rerender(
      <ErrorBoundary>
        <Boom throws={false} />
      </ErrorBoundary>,
    );
    await user.click(screen.getByRole("button", { name: "Try again" }));
    expect(screen.getByText("recovered")).toBeInTheDocument();
    expect(screen.queryByTestId("error-boundary")).not.toBeInTheDocument();
  });
});
