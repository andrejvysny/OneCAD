---
name: codex-astra
description: Delegates an explicitly-approved math, geometry, numerical-policy, or algorithm question to GPT-6 Astra via the Codex CLI in one of three modes — derive (design invariants, algorithm, or tolerance policy before code), break (adversarial review of a derivation, spec section, or diff excerpt), verify (check a proof, derivation, or test vectors). Prompt-only by default; Astra never explores the repo. Not for implementation, plan gating (codex-plan-review), or diff review (codex-implementation-review).
disable-model-invocation: true
---

# Codex Astra

Independent mathematical and geometric reasoning from GPT-6 Astra via `codex exec`, read-only, over a self-contained problem packet that Fable writes. Astra designs, attacks, or checks; it never implements, never explores the repository, and never gates a commit. Run only when the user explicitly invokes `/codex-astra <mode>`; Fable may *recommend* a run in a plan or a report, never start one on its own.

Sibling skills: `/codex-plan-review` gates a finished plan, `/codex-implementation-review` reviews a diff, `/codex-brainstorm` is a verdict-free thinking partner. This skill exists for questions whose answer is a formula, invariant, predicate, or algorithm, where the user's Plus limits make every Astra call expensive and the packet must therefore be precise.

## When it earns its cost

Astra runs on a ChatGPT Plus account with a five-hour and a weekly limit. Each call is a real budget. Recommend a run only when at least two of these hold; otherwise Fable derives inline.

- The deliverable is a formula, invariant, predicate, epsilon policy, frame, correspondence rule, or scoring function, not code.
- Being wrong would be silent: a mis-bind instead of `NeedsRepair`, an epsilon that accepts a bad blend, a frame that twists, a solver that converges to the wrong root.
- Fable's own derivation has a step it cannot verify by running something.
- The decision is expensive to reverse: it lands in `protocol/SCHEMA.md`, in `corpus/expected-values/`, or in kernel acceptance policy.

Not for: locating code, refactors, plumbing across layers, UI, plan gating, diff review, interactive QA, or anything a `ctest -R` probe answers faster.

## Modes

The first token after `/codex-astra` selects the mode.

| Mode | Purpose | Default effort |
|---|---|---|
| `derive` | Design invariants, predicates, an epsilon policy with its scaling law, degeneracy classes, an algorithm sketch, test vectors, and an implementation handoff, before code exists | `xhigh` |
| `break` | Adversarial review of a derivation, a spec section, or a kernel diff excerpt: counterexamples, proof gaps, missing degeneracy classes | `xhigh` |
| `verify` | Check a proof, a derivation, or a set of test vectors step by step; recompute the numbers | `high` |

`max` is allowed only with explicit approval, flagged as limit-heavy. It is unverified on Astra in this CLI: a rejected effort fails fast, so rerun at `xhigh` if it does. Never lower or raise effort silently.

## Limits policy

- One call per question. No multi-model fan-out, no second model on the same packet.
- One `derive` and one `break` per work package is the expected cadence, not one per task.
- Packet at most 40 KB. The model context is 272 K tokens in this CLI; the packet is the whole context Astra gets.
- Ask for compact output. Astra's tokens count against the same limit.
- On a quota or usage-limit error: stop, report the exact message, say which window (five-hour or weekly) it names. Do not retry, do not switch model.

## Access modes

State the chosen mode in the approval summary.

- **Prompt-only (default).** Astra reasons over the packet alone. Run from the session scratchpad directory with `--skip-git-repo-check`. This is the mode that fits the limits: Fable has already done the exploration.
- **Repository-grounded (approval only).** Astra may read an explicit list of at most five paths. Add `-C "<REPO_ROOT>"`, name the paths in the packet, keep `-s read-only`. `-C` sets the working directory; it is not a read allowlist, so keep the list in the prompt and say why each path is needed.

## Approval summary

Print this and wait for explicit approval before running:

```
Codex Astra
  Mode:         <derive | break | verify>
  Lens(es):     <from reference/lenses.md>
  Model:        gpt-6-astra
  Effort:       <high | xhigh | max*>   (*max needs explicit approval; unverified on Astra)
  Provider:     openai
  Access:       <prompt-only | repository-grounded: path list>
  Working dir:  <scratchpad | repo root>
  Web:          cached   (live only on approval)
  Packet:       <size in KB>, hash <sha256 first 12>
  Output file:  <scratchpad>/astra-<mode>-<topic>-<timestamp>.md
  Limit note:   one call; Plus five-hour + weekly windows apply
  Purpose:      <one sentence>
```

## Command

```bash
codex exec \
  -m "gpt-6-astra" \
  -c 'model_provider="openai"' \
  -c 'model_reasoning_effort="<EFFORT>"' \
  -c 'web_search="cached"' \
  -c 'approval_policy="never"' \
  -s read-only \
  --ephemeral \
  --strict-config \
  --ignore-user-config \
  --skip-git-repo-check \
  -o "<SCRATCHPAD>/astra-<mode>-<topic>-<timestamp>.md" \
  - <<'CODEXEOF'
<PACKET>
CODEXEOF
```

- `--ignore-user-config` drops the user's `model =` and sandbox defaults, so `-m "gpt-6-astra"` is mandatory.
- Repository-grounded: add `-C "<REPO_ROOT>"` and drop `--skip-git-repo-check`.
- Live web only on approval: `-c 'web_search="live"'`.
- A figure helps geometry questions: `-i <png>` attaches it.
- Never use `workspace-write`, `danger-full-access`, or `--dangerously-bypass-approvals-and-sandbox`.
- Compute the packet hash before the run: `shasum -a 256 <packet-file> | cut -c1-12`.

## Packet

Build the packet from `${CLAUDE_SKILL_DIR}/reference/packet.md`. It is XML-block structured: `<task>`, `<problem>`, `<invariants>`, `<already_tried>`, `<constraints>`, `<ask>`, then the grounding and output-contract blocks. Fill the mode's output contract from `${CLAUDE_SKILL_DIR}/reference/output-contract.md` and append the matching lens text from `${CLAUDE_SKILL_DIR}/reference/lenses.md`.

The packet must be self-contained: symbols defined, units stated, the current formula written out, measured residuals given with their provenance (a ctest name, a kernelbench row, a probe command). Write the packet to a scratchpad file first, check its size, then feed it through the heredoc. Astra cannot ask follow-up questions, so anything missing becomes a guess.

## After the run

1. Read the `-o` file. Treat it as evidence, not truth.
2. Verify every numeric claim Fable can check: a `ctest -R` probe, a kernelbench row, a scratch computation. Label each claim verified, refuted, or unverifiable.
3. Accept or reject each finding with a one-line reason.
4. Write the accepted derivation to `docs/design/astra/<topic>.md` using the header in `docs/design/astra/README.md`: date, mode, model, effort, packet hash, verdict, and what was rejected and why. That file is the provenance citation for any expected value derived from it. Unaccepted output stays in the scratchpad.
5. Hand off. A `derive` result becomes the DESIGN section of an implementation brief for `impl-critical` or `implementer`; Astra never writes code. A `break` result becomes red-first fixes. Both then go through the adversarial reviewer and the gate ladder as usual. An Astra derivation is evidence for a design, never for a gate.

## Failure policy

1. Report the exact error text.
2. Retry once with identical settings only for a transient network failure.
3. Quota or usage-limit: stop, report the window, do not retry.
4. Auth, missing CLI, unknown model, rejected effort, permission, or config errors: surface them; effort changes do not fix them.
5. Otherwise offer a clearly labelled local Fable derivation. Never present Fable's own derivation as the requested independent Astra result.

Treat as failures: empty output, a truncated output file, a resolved model other than `gpt-6-astra`, or a repository-grounded run that reports reading paths outside the approved list.

## Secrets and audit

Never put secrets, tokens, keys, `.env` contents, or production data in the packet. Do not direct Astra to credential files. If persistent logging is wanted, record metadata only: timestamp, session id, mode, model, effort, access mode, packet hash, exit status, duration. Never log packet contents or output.
