---
name: codex-astra
description: Delegates a math, geometry, numerical-policy, algorithm, prior-art, or algorithmic-cost question to GPT-6 Astra via the Codex CLI in one of six modes — derive (invariants, predicates, tolerance policy before code), break (adversarial review of a derivation, spec section, or kernel diff), verify (recheck a proof, derivation, or test vectors), research (prior art and documented OCCT behaviour, live web), perf (complexity and conditioning of a hot kernel path), followup (continue a previous Astra session with the answers it asked for). Read-only; Astra designs, attacks, or checks, and never writes code or closes a gate. Fable proposes the run and waits for approval. Not for implementation, plan gating (codex-plan-review), diff review (codex-implementation-review), or GUI QA.
---

# Codex Astra

Independent mathematical, geometric, and algorithmic reasoning from GPT-6 Astra via `codex exec`,
read-only, over a self-contained problem packet that Fable writes. Astra designs, attacks, or checks;
it never implements and never gates a commit.

> **Loading this skill is not approval to run.** Fable may load it and *propose* a run whenever the
> routing rule below fires — that is the expected behaviour, not an intrusion — but the `codex exec`
> call happens only after the user approves the printed summary. No silent runs, ever.

Sibling skills: `/codex-plan-review` gates a finished plan, `/codex-implementation-review` reviews a
diff, `/codex-brainstorm` is a verdict-free thinking partner on the GPT-5.6 line. This skill exists
for questions whose answer is a formula, invariant, predicate, algorithm, cost model, or a documented
external fact.

Account is **ChatGPT Pro (5×)**, so the binding constraint is judgement, not rationing: use Astra
whenever the routing rule fires, keep the packet honest, and verify every number that comes back.

## Routing rule

Propose an Astra run when the deliverable is one of these, rather than code:

- a formula, invariant, predicate, or proof obligation;
- an epsilon or tolerance policy together with its scaling law;
- a frame (sweep framing), a correspondence rule (loft), or a scoring function (topology identity);
- a degeneracy taxonomy — which configurations need their own branch and how each is detected;
- a complexity or conditioning claim about a kernel path;
- a documented external fact: what OCCT specifies, what a published algorithm assumes, what prior art
  already solved.

Also propose one when being wrong would be **silent**: a mis-bind instead of `NeedsRepair`, an
epsilon that accepts a bad blend, a frame that twists, a solver that converges to the wrong root; or
when the decision is expensive to reverse because it lands in `protocol/SCHEMA.md`, in
`corpus/expected-values/`, or in kernel acceptance policy.

Do **not** route here: locating code, refactors, plumbing across layers, UI work, plan gating, diff
review of non-kernel code, gate runs, or anything a `ctest -R` probe answers faster.

Interactive and GUI QA is **not** an Astra lane despite the model's computer-use strength: `codex
exec` is headless and exposes no computer-use tool. GUI QA stays Fable with Playwright and wdio. The
Codex Computer Use desktop app is a manual lane the user drives, not something this skill invokes.

## Modes

The first token after `/codex-astra` selects the mode.

| Mode | Purpose | Default effort | Default access |
|---|---|---|---|
| `derive` | Invariants, predicates, epsilon policy with its scaling law, degeneracy classes, algorithm sketch, test vectors, implementation handoff | `xhigh`; `max` for a new algorithm or a tolerance theory | grounded for a kernel question, else prompt-only |
| `break` | Adversarial attack on a derivation, a spec section, or a kernel diff excerpt | `xhigh` | grounded when it attacks a diff, else prompt-only |
| `verify` | Recheck a proof, a derivation, or a set of test vectors step by step; recompute the numbers | `high` | prompt-only |
| `research` | Prior art, published algorithms, documented OCCT behaviour; **live web** | `high` | prompt-only, live web |
| `perf` | Complexity, conditioning, and algorithmic cost of a hot kernel path, before Fable profiles | `high` / `xhigh` | grounded |
| `followup` | Answer the unknowns Astra named and continue the same session | inherits the parent run | inherits the parent run |

`followup` has no output contract of its own; it inherits the parent run's.

## Effort ladder

`gpt-6-astra` supports `low | medium | high | xhigh | max | ultra` (read off
`~/.codex/models_cache.json`; `default_reasoning_level` is `medium`, which is below what any mode
here wants).

- `high` — `verify`, `research`, a bounded `perf`.
- `xhigh` — the normal rung for `derive` and `break`.
- `max` — a normal choice for a `derive` that invents an algorithm or a tolerance theory. No extra
  approval beyond the run itself.
- `ultra` — **explicit approval only.** It is multi-agent (`multi_agent_version v2`,
  `multi_agent_reasoning_effort xhigh`): it delegates to sub-agents, so one `ultra` call is not one
  call's worth of quota. Say that in the summary when proposing it. Reserve it for the hardest
  derivation in a program, not for a hard one.

Never change effort silently in either direction.

## Budget policy

- About **three calls per work package** — typically `derive`, `break`, and one `followup` or a
  second `break` on a different dimension. A fourth is allowed with one sentence saying why.
- Parallel fan-out at most **2**, and only on genuinely **distinct** questions. Never the same packet
  to two models, and never Astra plus a GPT-5.6 sibling on the same question.
- Packet at most **120 KB**. Context is 272 000 tokens (`max_context_window` 872 000), so 120 KB is
  roughly a tenth of the cheap window — the cap exists to keep the packet readable, not to fit it.
- Still ask for compact output; a recap of the packet is wasted reasoning.
- On `rate_limit_reached`, a usage-limit error, or a credits-depleted error: stop, quote the exact
  message, name the window it reports (five-hour or weekly), do not retry, do not switch model.

## Access modes

State the chosen mode in the approval summary.

- **Prompt-only** — the default for anything that is not a kernel question. Astra reasons over the
  packet alone. Run from the session scratchpad directory with `--skip-git-repo-check`.
- **Repository-grounded** — the default for a *kernel* question: one about `worker/src/kernel/**`,
  `worker/src/ops/**`, a named `protocol/SCHEMA.md` section, `src-tauri/crates/onecad-kernelbench`,
  or a kernelbench result set. Add `-C "<REPO_ROOT>"`, drop `--skip-git-repo-check`, keep
  `-s read-only`.
  - Name **at most 12** paths in the packet's `<repo_paths>` block, one clause each on why it is
    needed. `-C` sets a working directory; it is not a read allowlist, so the list binds only because
    the packet says so — check after the run that the transcript read nothing outside it.
  - Do **not** allowlist `corpus/expected-values/**` on a `derive`. A derivation that can read the
    expected answer is shaped by it; transcribe the measured numbers into `<problem>` with their
    provenance instead. A `verify` or `break` may read it when the question is precisely whether the
    recorded value is right.
  - Never allowlist `.env`, credentials, `~/.codex`, `~/.claude`, or any home-directory config path.

## Approval summary

Print this and wait for explicit approval before running:

```
Codex Astra
  Mode:         <derive | break | verify | research | perf | followup>
  Lens(es):     <from reference/lenses.md>
  Model:        gpt-6-astra
  Effort:       <high | xhigh | max | ultra*>   (*ultra needs explicit approval; multi-agent, burns more than one call)
  Provider:     openai
  Access:       <prompt-only | grounded: path list>
  Working dir:  <scratchpad | repo root>
  Web:          <cached | live (research)>
  Packet:       <size in KB>, hash <sha256 first 12>
  Resumable:    <no (--ephemeral) | yes, session recorded for followup>
  Output file:  <scratchpad>/astra-<mode>-<topic>-<timestamp>.md
  Budget:       call <n> of ~3 for <work package>
  Purpose:      <one sentence>
```

## Command

Base form — prompt-only, not resumable:

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

Variants, applied on top of the base form:

- **grounded** — add `-C "<REPO_ROOT>"`, remove `--skip-git-repo-check`. Keep `-s read-only`.
- **research** — replace `web_search="cached"` with `-c 'web_search="live"'`, and require a citation
  per external claim in the packet.
- **followup-eligible** — remove `--ephemeral` (it persists no session files, so there is nothing to
  resume) and add `--json > "<SCRATCHPAD>/astra-<topic>-events.jsonl"` alongside `-o`, then read the
  session id out of that JSONL.

  The id is the `thread_id` of the JSONL's **first** line, `{"type":"thread.started","thread_id":"..."}`:

  ```bash
  TID=$(head -1 "<SCRATCHPAD>/astra-<topic>-events.jsonl" | python3 -c 'import json,sys;print(json.load(sys.stdin)["thread_id"])')
  ```

  Then continue the session. **`codex exec resume` accepts no `-s` flag** — it rejects it with
  `error: unexpected argument '-s' found`, so the sandbox goes through a config override instead:

  ```bash
  codex exec resume "$TID" \
    -m "gpt-6-astra" -c 'model_provider="openai"' \
    -c 'model_reasoning_effort="<EFFORT>"' -c 'approval_policy="never"' \
    -c 'sandbox_mode="read-only"' \
    --strict-config --ignore-user-config --skip-git-repo-check \
    --json > "<SCRATCHPAD>/astra-followup-<topic>-events.jsonl" \
    -o "<SCRATCHPAD>/astra-followup-<topic>-<timestamp>.md" \
    - <<'CODEXEOF'
  <FOLLOWUP PACKET: the answers to the unknowns Astra named, nothing else restated>
  CODEXEOF
  ```

  `codex exec resume --last` is a fragile fallback: it picks the newest recorded session, which is
  the wrong one if any other `codex` run happened in between. Prefer the explicit id.

  **Verified 2026-09-11** against codex-cli 0.153.4 on a `verify` run at `low` effort: the model slug
  resolves under `--strict-config`, the contract sections come back in order, the planted wrong number
  was caught and recomputed, and the resumed turn still held the parent packet (it recalled the
  geometry and returned only the step that changed). Cost for the pair: 20 s / 415 output tokens for
  the parent, 9 s / 80 output tokens for the followup, 7 040 cached input tokens reused on the
  resume.

Other flags:

- `--ignore-user-config` drops the user's `model =` and sandbox defaults, so `-m "gpt-6-astra"` is
  mandatory.
- A figure helps geometry questions: `-i <png>` attaches it.
- `--output-schema <FILE>` exists if a machine-checkable answer is ever wanted; the contracts here
  are prose and do not use it.
- Do not add `model_verbosity` or `service_tier` overrides — `--strict-config` fails on an
  unrecognised key and neither was confirmed against this CLI build (codex-cli 0.153.4).
- Never use `workspace-write`, `danger-full-access`, or
  `--dangerously-bypass-approvals-and-sandbox`.
- Compute the packet hash before the run: `shasum -a 256 <packet-file> | cut -c1-12`.

## Packet

Build the packet from `${CLAUDE_SKILL_DIR}/reference/packet.md`. It is XML-block structured:
`<task>`, `<problem>`, `<invariants>`, `<already_tried>`, `<constraints>`, `<ask>`, plus
`<repo_paths>` in grounded mode, then the grounding and output-contract blocks. Fill the mode's
output contract from `${CLAUDE_SKILL_DIR}/reference/output-contract.md` and append the matching lens
text from `${CLAUDE_SKILL_DIR}/reference/lenses.md`.

The packet must be self-contained even in grounded mode: symbols defined, units stated, the current
formula written out, measured residuals given with their provenance (a ctest name, a kernelbench row,
a probe command). Grounding lets Astra check the packet against source; it does not excuse a packet
that makes Astra go looking for the question. Write the packet to a scratchpad file first, check its
size, then feed it through the heredoc.

## After the run

1. Read the `-o` file. Treat it as evidence, not truth.
2. Verify every numeric claim Fable can check: a `ctest -R` probe, a kernelbench row, a scratch
   computation. Label each claim verified, refuted, or unverifiable.
3. Accept or reject each finding with a one-line reason.
4. Write the accepted derivation to `docs/design/astra/<topic>.md` using the header in
   `docs/design/astra/README.md`: date, mode, model, effort, access, packet hash, call number,
   verdict, and what was rejected and why. That file is the provenance citation for any expected
   value derived from it. Unaccepted output stays in the scratchpad.
5. Hand off. A `derive` result becomes the DESIGN section of an implementation brief for
   `impl-critical` or `implementer`; Astra never writes code. A `break` result becomes red-first
   fixes. Both then go through the adversarial reviewer and the gate ladder as usual. An Astra
   derivation is evidence for a design, never for a gate.

Mode-specific:

- **research** — accept a claim only when its citation is one Fable can open and read. An uncited
  claim is recorded as `inference` or dropped; it never becomes a repository fact.
- **perf** — the result is a design input. Any number that lands in a report still comes from
  kernelbench or a ctest timing, never from Astra's cost model.
- **unknowns** — if Astra names an unknown that would change the answer, the cheap move is now a
  `followup` round, not a guess by either model.

## Failure policy

1. Report the exact error text.
2. Retry once with identical settings only for a transient network failure.
3. Quota, usage-limit, or credits-depleted: stop, report the window, do not retry.
4. Auth, missing CLI, unknown model, rejected effort, permission, or config errors: surface them;
   effort changes do not fix them.
5. Otherwise offer a clearly labelled local Fable derivation. Never present Fable's own derivation as
   the requested independent Astra result.

Treat as failures: empty output, a truncated output file, a resolved model other than
`gpt-6-astra`, a grounded run whose transcript reports reading paths outside the approved list, and
an `ultra` run that returns a delegation summary instead of the contract sections (re-run at `max`).

## Secrets and audit

Never put secrets, tokens, keys, `.env` contents, or production data in the packet, and never
allowlist a credential path in grounded mode. If persistent logging is wanted, record metadata only:
timestamp, session id, mode, model, effort, access mode, packet hash, call number, exit status,
duration. Never log packet contents or output.
