/**
 * The tool set this build ships: one read-only tool, and nothing that touches
 * the world.
 *
 * There is no filesystem tool, no shell tool, no network tool and no CAD tool
 * here, and that is a property of the build rather than an oversight. The
 * assistant's authority is exactly the union of what these tools can do and
 * what `docs/assistant/wire-protocol.md` §5's verb table allows, and both
 * halves are deliberately empty of side effects. A tool that reads a OneCAD
 * document — let alone writes one — is a work package with its own
 * authorization design; it arrives here together with a real
 * `ProposalApplier` (see `../composition/noopApplier.ts`), not before.
 *
 * What this file is for in the meantime is the round trip: it proves that the
 * catalogue route, the staging path and a turn all see the same tool, so the
 * first real tool has somewhere known-good to land.
 */
import { CONTRACT_VERSION } from "agentkit/contracts";
import type { AiTool, AiToolExecutionContext } from "agentkit/core";
import type { ToolContributionContext, ToolSetContributor } from "agentkit/host";
import { OCAK_PROTOCOL_VERSION } from "../bridge/peer.js";

/**
 * `onecad`, not `agentkit` / `chat` / `mcp` — those three are reserved by the
 * framework and staging refuses them. The namespace is attribution and
 * reservation, not a prefix: it does not rename the tools below, it just means
 * a second contributor cannot quietly shadow one of them.
 */
export const TOOL_NAMESPACE = "onecad";

interface HostInfo {
  bridgeProtocolVersion: number;
  agentkitContractVersion: string;
  documentMutation: "unavailable";
}

/**
 * `onecad_host_info`, not `onecad.host.info`: `AiToolRegistry.register` rejects
 * a dotted tool name (`TOOL_NAME_PATTERN` is `^[a-zA-Z0-9_-]+$`). The dotted
 * identifier lives in `capability`, which has no such restriction.
 *
 * It answers from constants, so it can never fail, never blocks, and gives the
 * model a truthful answer to "can you change my model?" — which is the one
 * thing worth being unambiguous about in a build that cannot.
 */
const hostInfoTool: AiTool<Record<string, never>, HostInfo> = {
  definition: {
    name: "onecad_host_info",
    version: "1.0.0",
    effect: "read",
    capability: "onecad.host.info",
    description:
      "Report the OneCAD assistant host's bridge protocol version, AgentKit contract version, " +
      "and whether it can mutate the open document.",
    inputSchema: {
      type: "object",
      properties: {},
      additionalProperties: false,
    },
  },
  async execute(ctx: AiToolExecutionContext) {
    const data: HostInfo = {
      bridgeProtocolVersion: OCAK_PROTOCOL_VERSION,
      agentkitContractVersion: CONTRACT_VERSION,
      documentMutation: "unavailable",
    };
    return {
      ok: true,
      data,
      summary: `OCAK1 v${OCAK_PROTOCOL_VERSION}, AgentKit ${CONTRACT_VERSION}, no document mutation`,
      sources: [],
      warnings: [],
      truncated: false,
      limits: ctx.limits,
    };
  },
};

export function createOneCadToolSetContributor(): ToolSetContributor {
  const tools = [hostInfoTool] as unknown as AiTool[];
  return {
    namespace: TOOL_NAMESPACE,
    async contribute(_ctx: ToolContributionContext): Promise<AiTool[]> {
      return tools;
    },
  };
}
