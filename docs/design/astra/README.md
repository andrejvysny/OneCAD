# Astra derivations

Accepted output of `/codex-astra` runs (GPT-6 Astra via `codex exec`, read-only). Each file is the
provenance citation for any expected value, threshold, acceptance policy, or cost claim derived from
it; cite it the same way a `corpus/` recording is cited.

Only accepted content lands here. Rejected findings are named in the header with the reason; the raw
output stays in the session scratchpad.

File name: `<topic>.md`, kebab-case, usually the work package, for example `wp-g-fillet-envelope.md`.

Header:

```
# <topic>
date: YYYY-MM-DD
mode: derive | break | verify | research | perf | followup
model: gpt-6-astra
effort: high | xhigh | max | ultra
access: prompt-only | grounded: <paths read>
packet: sha256 <first 12>
calls: <n> of ~3 for <work package><, resumed from session <id> for a followup>
verdict: <Astra's verdict, or "n/a" for derive, research, perf>
verified by: <what Fable ran to check the numbers: ctest names, kernelbench rows, probes, scratch computations>
rejected: <finding and reason, one line each, or "none">
```

Body: the accepted sections, in the order of the mode's output contract, edited only to remove
rejected material.

A `research` file keeps its citations inline — a claim whose source Fable could not open does not
belong in this directory. A `perf` file is a design input; the timing numbers in a gate report still
come from kernelbench or a ctest.
