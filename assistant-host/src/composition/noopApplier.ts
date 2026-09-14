/**
 * The structural expression of this build's no-mutation guarantee.
 *
 * `ProposalService` needs a `ProposalApplier` — `recoverOnBoot` reconciles
 * against one — but in this build there is nothing for it to apply. The
 * assistant has **no document mutation authority**: it ships no write tools,
 * and `docs/assistant/wire-protocol.md` §5 deliberately has no verb by which
 * this process could change a OneCAD document even if it had one.
 *
 * The guarantee is expressed as an object that throws rather than as a comment
 * that asks, because a comment cannot fail a test. If a write path is ever
 * wired by accident, it fails here, loudly, with a message that names the
 * invariant it broke — instead of succeeding and quietly giving a language
 * model the edit history.
 *
 * **This is the one object a later work package replaces.** When the assistant
 * is given a mutation path, it arrives as a real applier here, behind the
 * approval pipeline (`SessionWritePolicy` + `ProposalService`) that is already
 * wired in `./app.ts`. Nothing else in this package has to change for that, and
 * nothing else in this package should be where the decision is made.
 */
import type { ApplyOutcome, ApplyProposalInput, ProposalApplier } from "agentkit/host";

/** Thrown by {@link NoopProposalApplier.apply}. Typed so a test can assert on it. */
export class NoDocumentAuthorityError extends Error {
  override readonly name = "NoDocumentAuthorityError";
  constructor(readonly proposalId: string) {
    super("assistant has no document mutation authority in this build");
  }
}

export class NoopProposalApplier implements ProposalApplier {
  async apply(input: ApplyProposalInput): Promise<ApplyOutcome> {
    throw new NoDocumentAuthorityError(input.proposal.id);
  }

  /**
   * `null`, not a synthesised failure record: "I have never applied anything"
   * is the true answer, and a fabricated outcome would let `recoverOnBoot`
   * believe a write happened.
   */
  async getOutcome(_operationId: string): Promise<ApplyOutcome | null> {
    return null;
  }
}
