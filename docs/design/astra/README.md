# Astra derivations

Accepted output of `/codex-astra` runs (GPT-6 Astra via `codex exec`, read-only, prompt-only). Each file is the provenance citation for any expected value, threshold, or acceptance policy derived from it; cite it the same way a `corpus/` recording is cited.

Only accepted content lands here. Rejected findings are named in the header with the reason; the raw output stays in the session scratchpad.

File name: `<topic>.md`, kebab-case, usually the work package, for example `wp-g-fillet-envelope.md`.

Header:

```
# <topic>
date: YYYY-MM-DD
mode: derive | break | verify
model: gpt-6-astra
effort: high | xhigh | max
packet: sha256 <first 12>
verdict: <Astra's verdict or "n/a" for derive>
verified by: <what Fable ran to check the numbers: ctest names, probes, scratch computations>
rejected: <finding and reason, one line each, or "none">
```

Body: the accepted sections, in the order of the mode's output contract, edited only to remove rejected material.
