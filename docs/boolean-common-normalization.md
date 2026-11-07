# Auxiliary topology in Boolean Common

Standalone Boolean Intersect first runs the ordinary publication policy. A
publishable result retains its original shape, history, and topology ordinals.
Only a refused result with checked open edges and zero non-manifold edges can
enter auxiliary-topology normalization. Cut, Union, and tool pre-combination
retain their existing behavior.

The normalizer copies both topology and geometry before calling OCCT's
`BRepTools::RemoveInternals` with `force=false`. It admits only INTERNAL wire,
edge, and vertex occurrences, including a nonempty wire containing exclusively
INTERNAL edges. INTERNAL faces and EXTERNAL occurrences are unsupported. OCCT
uses orientation and connectivity here, not geometric classification.

Before cleanup, the copy's external boundary is captured as a multiset of parent
and child identities, locations, and local orientations. Cleanup must preserve
that graph and the root identity exactly. New or replaced external entities,
changed external uses, unsupported topology, cancellation, or OCCT failure leave
the original result and history untouched. The normalizer never changes a
tolerance or uses mass equality as its acceptance rule.

History composes the original Boolean and the pre-cleanup copy mapping through
`elementmap::ComposedHistory`. Final-shape filtering excludes removed auxiliary
images. Missing images without positive deletion evidence remain unresolved and
can produce `NeedsRepair`; they do not silently bind to another boundary.

The candidate then runs the same publication policy, tolerance ceiling,
self-interference, manifold, lifecycle, and resource checks. Only that final
decision can publish. Cancellation is checked during traversal and between
phases; the OCCT copy and cleanup calls themselves are not interruptible.

`worker/tests/test_common_internals.cpp` exercises the captured native Common
failure, input isolation, repeatability, actual Boolean publication, retained
tools, history, cancellation, and orientation cases. Fixture provenance lives
beside its operands in `worker/tests/fixtures/common-internals`. Fixed material
classification samples and volume comparisons are behavior regressions, not an
independent Framework source-equivalence oracle or complete workflow acceptance.
