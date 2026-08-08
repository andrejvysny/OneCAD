/*
 * Per-contribution error boundary.
 *
 * A throw inside one contributed panel must not take down the editor. The
 * app-level `ErrorBoundary` renders a full-screen recovery page, which is the
 * wrong shape here — a broken 260px panel should fail inside its own footprint
 * and leave the rest of the application usable (spec §73/§129).
 *
 * Must be a CLASS: `getDerivedStateFromError` has no hook equivalent.
 */
import { Component, type ErrorInfo, type ReactNode } from "react";
import { logError } from "@/debug/log";

interface Props {
  /** Contribution id, used in the log line and the fallback text. */
  contributionId: string;
  children: ReactNode;
}

interface State {
  error: Error | null;
}

export class ContributionBoundary extends Component<Props, State> {
  state: State = { error: null };

  static getDerivedStateFromError(error: Error): State {
    return { error };
  }

  componentDidCatch(error: Error, info: ErrorInfo): void {
    logError("err", `contribution crashed: ${this.props.contributionId}`, {
      contributionId: this.props.contributionId,
      name: error.name,
      message: error.message,
      componentStack: info.componentStack ?? undefined,
    });
  }

  render(): ReactNode {
    const { error } = this.state;
    if (error === null) return this.props.children;
    return (
      <div
        role="alert"
        data-testid="contribution-error"
        data-contribution-error={this.props.contributionId}
        className="m-2 rounded-md border border-warn-border bg-warn-surface px-3 py-2 text-[12px] text-warn-strong"
      >
        This panel failed to render.
      </div>
    );
  }
}
