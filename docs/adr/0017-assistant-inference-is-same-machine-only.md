# 0017 — Assistant inference is same-machine only

**Status:** Accepted (2026-09-14)
**Depends on:** ADR-0015, ADR-0016.

## Context

The product requirement is that the assistant runs fully locally. Read strictly, that
covers model inference, not only orchestration: an assistant that ships its user's CAD
geometry to a hosted model is not local because the chat loop happens to run on the desktop.

## Decision

All product inference runs on the same machine. The assistant reaches a model **only**
through a Rust-owned local-provider gateway that accepts a **registered provider id and a
bounded payload — never a URL chosen by the model or by the sidecar.**

The gateway:

- allows only literal loopback addresses and the registered port and path family;
- normalises `localhost` to a chosen loopback address at settings-validation time, so the
  check happens once against a canonical form rather than per-request against a name;
- rejects encoded and otherwise ambiguous host forms;
- disables HTTP redirects and refuses to inherit proxy environment variables;
- enforces size, streaming and cancellation limits.

Provider configuration changes only through a trusted UI command. A model response cannot
change its own base URL, enable network access, or select a different endpoint.

## What this decision does NOT claim

**This is application policy over code we ship, not an OS sandbox.** It constrains a
cooperating sidecar and a misbehaving model. It does not constrain an arbitrarily
compromised child process, and it cannot: a process that can open a socket can open a
socket. Where process-level network restrictions are available they strengthen the claim;
the honest gate is an offline acceptance test with non-loopback networking disabled.

**A loopback endpoint is not proof of local inference.** A local server can proxy a cloud
model. The gateway cannot detect that, and no amount of URL validation will. Only running
the whole job with external networking disabled, against known local weights, establishes
it — which is why that is a required release gate and not a code check.

## GPT-6 Astra

Astra is documented as an API model. The documentation reviewed establishes no downloadable
local runtime or weights, so Astra must not be presented as a locally runnable dependency
of this product, and a remote Astra integration is outside this release's scope.

This says nothing about using Astra as a *coding* assistant to build OneCAD — that is a
development tool, not the product's runtime data flow, and the two are unrelated.

## Consequences

The provider interface stays extensible, and a future non-local option remains
implementable — but it must arrive as a deliberate, visible product decision with its own
ADR. The implementing agent is explicitly instructed not to introduce cloud inference as a
fallback, silent or otherwise.

The user installs and runs their own inference runtime for now. AgentKit itself is bundled,
so no globally installed Bun, Node, or CLI agent is ever required.
