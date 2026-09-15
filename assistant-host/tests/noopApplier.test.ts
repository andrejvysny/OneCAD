import { describe, expect, test } from "bun:test";
import {
  NoDocumentAuthorityError,
  NoopProposalApplier,
} from "../src/composition/noopApplier.js";
import type { ApplyProposalInput } from "agentkit/host";

const input = {
  proposal: { id: "proposal_1" },
  operationId: "op_1",
} as unknown as ApplyProposalInput;

describe("the no-mutation guarantee", () => {
  test("apply throws, and names the invariant rather than a generic failure", async () => {
    const applier = new NoopProposalApplier();
    await expect(applier.apply(input)).rejects.toThrow(NoDocumentAuthorityError);
    await expect(applier.apply(input)).rejects.toThrow(
      "assistant has no document mutation authority in this build",
    );
  });

  test("the thrown error carries the proposal it refused", async () => {
    const applier = new NoopProposalApplier();
    const failure = await applier.apply(input).catch((err: unknown) => err);
    expect(failure).toBeInstanceOf(NoDocumentAuthorityError);
    expect((failure as NoDocumentAuthorityError).proposalId).toBe("proposal_1");
  });

  test("getOutcome answers null rather than fabricating a record", async () => {
    expect(await new NoopProposalApplier().getOutcome("op_1")).toBeNull();
  });
});
