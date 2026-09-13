# OneCAD professional viewport — implementation package

**Date:** 13 September 2026  
**Version:** VP-HARDENING 1.0  
**Implementer/orchestrator:** Claude Code with Fable 5.1  
**Repository baseline:** `andrejvysny/OneCAD` → `65b4c60eb226a201f2fa3eb565d7283b1c63b5ac`

## What this package is

A selected technical design and an implementation contract for professional-quality model rendering, shading, edges, sketches, navigation, picking, sections, and previews. The architecture and difficult numerical choices are already specified. Fable implements, integrates, and verifies them; it does not need to invent the design.

This package does not contain application patches and does not claim that OneCAD's native/GPU tests have run. All performance and quality thresholds are new acceptance targets. The original static review is preserved as historical evidence, not a current pass/fail ledger.

## Documents

| Read order | Document | Purpose |
|---|---|---|
| 1 | [01-SPECIFICATION.md](01-SPECIFICATION.md) | Normative behavior, 21 requirement groups, module responsibilities, defaults and budgets |
| 2 | [03-NUMERICS-AND-PROTOCOL.md](03-NUMERICS-AND-PROTOCOL.md) | Exact curve strategy, projected error, normals, picking/camera math, local origins, binary-version migration |
| 3 | [02-IMPLEMENTATION-GUIDE.md](02-IMPLEMENTATION-GUIDE.md) | 17 work packages WP00–WP16, dependency graph, file-level steps, commands and exits |
| 4 | [04-ACCEPTANCE-AND-TESTS.md](04-ACCEPTANCE-AND-TESTS.md) | Fixture catalog, stable test IDs, measurement methods, native and physical gates |
| 5 | [06-DECISIONS-AND-TRACEABILITY.md](06-DECISIONS-AND-TRACEABILITY.md) | Fixed architecture decisions and R01–R18 coverage |
| 6 | [05-CLAUDE-ORCHESTRATOR.md](05-CLAUDE-ORCHESTRATOR.md) | Ready-to-use main implementation prompt |
| 7 | [07-EXECUTION-LEDGER-TEMPLATE.md](07-EXECUTION-LEDGER-TEMPLATE.md) | Status, file ownership, heavy-lane lease, test evidence, deviations and handoff |
| Reference | [08-SOURCES-AND-BASELINE.md](08-SOURCES-AND-BASELINE.md) | Primary sources, exact baseline facts and evidence limits |
| Reference | [Design arithmetic checks](reference/design-math-checks.md) | Executed isolated equation checks; explicitly not application acceptance |
| Reference | [Original rendering review](reference/original-rendering-review.md) | Unmodified review input, including R01–R17 |

Package checksums and file sizes are listed in [MANIFEST.md](MANIFEST.md).

Optional agent definitions are in `agent-templates/`. They are implementation aids, not additional architectural authority.

## Launch in Claude Code

Place this directory's contents under `docs/viewport-hardening/` inside the existing OneCAD checkout. Do not overwrite existing files without comparing them. This documentation step does not authorize Git commits, pushes, resets, branch changes, or cleanup.

Open Claude Code at the repository root. Select **Fable 5.1** using the model selector available in your installed setup. Confirm the active model; do not copy an unverified model identifier from another provider. Use ordinary permission controls.

Give Claude this kickoff instruction:

```text
Implement the OneCAD VP-HARDENING 1.0 package in docs/viewport-hardening/.
Read 00-START-HERE.md and follow 05-CLAUDE-ORCHESTRATOR.md as your main task.
The specification, numerical contracts, and implementation guide decide the design.
Inspect the current checkout, preserve all existing work, and execute WP00 first.
Then implement the work packages in dependency order. Maintain the execution ledger.
Do not stop at another plan. Do not commit/push/reset or claim native acceptance
without the required authority and evidence.
```

For more direct control, paste the task body from `05-CLAUDE-ORCHESTRATOR.md` instead. It is self-contained about role, constraints, fixed decisions, delegation, and completion.

## Optional specialist agents

Copy the relevant template files into the repository's `.claude/agents/` directory only when a same-named file does not already exist. Review and merge conflicts rather than overwriting. These Markdown frontmatter definitions use `model: inherit`, so verify observed model routing when used. No special multi-agent framework or external API is required. [EXT-07 in the sources document.]

The reviewer template is source-read-only. The main orchestrator runs the serialized gate commands and records their output. A Bash-capable agent can modify files through shell commands; withholding Edit/Write alone is not a complete read-only security boundary. The supplied reviewer therefore has no Bash tool.

## Boundaries that matter

The four milestones are: **M1 correctness baseline**, **M2 geometric fidelity**, **M3 integrated viewport**, and **M4 qualification**. Do not call M1 the full delivery. Physical-device or native-environment gaps remain explicit, even when code and unit tests are complete.

The package preserves the current stack and Z-up coordinates. It deliberately does not include a renderer rewrite, photorealistic rendering, product-wide feature parity, a tablet product, or production WebGPU. Long-term quality here means explicit contracts, bounded work/resources, sound geometry approximations, predictable interaction, and reproducible evidence.

A newly discovered problem, R18, corrects authored face colors using face ordinals rather than triangle indices. It is included in WP05 and the acceptance suite; the original review remains unchanged.
