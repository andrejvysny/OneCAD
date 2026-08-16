/**
 * The backend error envelope, and the guards the UI branches on.
 *
 * Rust's `ApiError` serializes as `{kind, message, diagnostics?}` and Tauri rejects
 * an `invoke` with that OBJECT — not an `Error` instance. So a `catch` in the
 * frontend sees a bare record on the tauri lane and, historically, an `Error` on the
 * mock lane. Anything that wants to branch on `kind` had to know that, and code that
 * did not simply lost the distinction.
 *
 * {@link BackendError} lets the mock throw the same shape while still being a real
 * `Error` (so a stack and a readable message survive), and {@link errorKind} reads
 * `kind` off either form.
 */

/** An error carrying a backend `ApiError` kind — the mock lane's throw. */
export class BackendError extends Error {
  constructor(
    readonly kind: string,
    message: string,
  ) {
    super(message);
    this.name = "BackendError";
  }
}

/**
 * The backend `ApiError.kind` behind a caught value, or `undefined` when it is not
 * a backend error. Handles both lanes: the tauri client's plain rejected object and
 * the mock's {@link BackendError}.
 */
export function errorKind(error: unknown): string | undefined {
  if (typeof error !== "object" || error === null) return undefined;
  const kind = (error as { kind?: unknown }).kind;
  return typeof kind === "string" ? kind : undefined;
}

/** A human-readable message from a caught backend error. */
export function errorMessage(error: unknown): string {
  if (typeof error === "object" && error !== null) {
    const message = (error as { message?: unknown }).message;
    if (typeof message === "string") return message;
  }
  return String(error);
}

/**
 * Whether opening a path was refused because unresolved crash-recovery work names
 * it — the file on disk is OLDER than an autosave a previous session left behind.
 * The caller must ask the user before proceeding: opening the saved version
 * discards that work irreversibly.
 */
export function isRecoveryPending(error: unknown): boolean {
  return errorKind(error) === "recoveryPending";
}
