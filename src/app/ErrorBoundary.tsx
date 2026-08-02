/*
 * ErrorBoundary — the last line of the frontend error lane.
 *
 * A render/lifecycle throw anywhere in the tree unmounts the whole app to a
 * blank page and React only prints to the console; nothing reaches `dev.jsonl`
 * and nothing tells the user. This catches it, logs it structurally (with the
 * React `componentStack`, which no other lane carries) and shows a recoverable
 * fallback.
 *
 * Must be a CLASS: `getDerivedStateFromError`/`componentDidCatch` have no hook
 * equivalent. "Try again" clears the error so a transient failure (a stale
 * projection, a mid-flight store swap) does not need an app restart.
 */
import { Component, type ErrorInfo, type ReactNode } from "react";
import { logError } from "@/debug/log";

interface ErrorBoundaryProps {
  children: ReactNode;
}

interface ErrorBoundaryState {
  error: Error | null;
}

export class ErrorBoundary extends Component<ErrorBoundaryProps, ErrorBoundaryState> {
  state: ErrorBoundaryState = { error: null };

  static getDerivedStateFromError(error: Error): ErrorBoundaryState {
    return { error };
  }

  componentDidCatch(error: Error, info: ErrorInfo): void {
    logError("err", `react tree crashed: ${error.message}`, {
      name: error.name,
      stack: error.stack,
      componentStack: info.componentStack ?? undefined,
    });
  }

  private readonly retry = (): void => {
    this.setState({ error: null });
  };

  render(): ReactNode {
    const { error } = this.state;
    if (error === null) return this.props.children;
    return (
      <div
        role="alert"
        data-testid="error-boundary"
        className="flex h-full w-full flex-col items-center justify-center gap-3 bg-canvas p-8 text-ink"
      >
        <h1 className="text-base font-medium">Something went wrong</h1>
        <p className="max-w-prose text-center text-sm text-ink-4">{error.message}</p>
        <button
          type="button"
          onClick={this.retry}
          className="rounded-md border border-border bg-surface px-3 py-1.5 text-sm text-ink-2 hover:bg-hover"
        >
          Try again
        </button>
      </div>
    );
  }
}
