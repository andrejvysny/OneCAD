# Output contracts

Paste the block for the chosen mode into `<structured_output_contract>` and mirror its numbering in `<ask>`.

## derive

```
1. Problem restatement in your own terms; surface any misframing in one line.
2. Properties to guarantee: the invariants the policy must hold, each as a checkable statement.
3. Predicates and epsilon derivation: for each acceptance test give the predicate, the epsilon,
   and its scaling law (absolute | relative to size | curvature-relative | kernel-tolerance-relative).
   State how each behaves under a similarity transform and under a unit change.
4. Degeneracy classes: every configuration that needs its own branch rather than the generic
   epsilon (zero curvature, tangent supports, seam edges, near-parallel axes, ...), with the
   detection predicate for each.
5. Algorithm sketch: steps, inputs, outputs, complexity, where each predicate is evaluated.
6. Test vectors: concrete inputs with expected outputs, at least one per degeneracy class and one
   near each threshold, each with the computation that produced the expected value.
7. Counterexample search: the strongest configuration that could defeat the policy, and whether it
   does. Show the numbers.
8. Implementation handoff: interfaces (function signatures and types), where each predicate lives
   (worker op, range analyzer, Rust planner), what to log, what diagnostic to publish on refusal.
9. Confidence (high | medium | low) and the unknowns that would change the answer.
```

## break

```
1. Verdict: sound | sound-with-conditions | defective, one sentence.
2. Findings, ranked by severity. For each: the claim attacked; the counterexample (with numbers)
   or the proof gap; severity (blocker | high | medium | low); basis (proved | numeric | inference);
   the smallest change that closes it.
3. Missing degeneracy classes the derivation does not handle, with a detection predicate each.
4. What a test would have to assert to catch each finding, and the input that triggers it.
5. Verified versus inferred: two lists, so the reader knows what you actually computed.
6. If nothing qualifies, say so and list what you checked.
```

Severity: blocker means the policy accepts a wrong result or refuses a valid one in a reachable configuration; high means a reachable configuration is mishandled but the core approach stands; medium means a bounded weakness; low means a concrete minor improvement.

## verify

```
1. Step table: for each step of the derivation, holds | fails | unverifiable, with the check you ran.
2. Recomputed test vectors: your value beside the claimed value, with the computation; flag any
   disagreement beyond the stated precision.
3. Minimal fix for each failing step, or "no fix needed".
4. Anything the derivation assumes without stating.
```
