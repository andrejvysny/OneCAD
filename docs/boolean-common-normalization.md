# Auxiliary topology in Boolean Common and Union

Standalone Boolean Intersect and Union first run the ordinary publication policy. A
publishable result retains its original shape, history, and topology ordinals.
Only a refused result with checked open edges and zero non-manifold edges can
enter auxiliary-topology normalization. Cut and tool pre-combination retain their existing behavior.

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

Union additionally declares the exact kernel tool shapes as history inputs,
including private copies for retained tools. For every input face, its raw
Modified or unchanged surviving images must map exactly through the copy to the
composed history's final face images. Generated images never establish tool
participation. Target anchoring and unique tool contribution checks still apply
after normalization; no absent or shared contribution is manufactured.

`test_union_internals.cpp` explicitly selects eight diagnostic operands (all
except operand 2) from `fixtures/union-internals`. It checks actual publication,
retained/consumed ownership, target face/edge/vertex successor keys, positive raw
deletion evidence, exact unaffected-reference snapshots, cancellation during
lineage construction, and byte-identical already-accepted raw Union behavior.
An independent replay uses the same retained-tool copy policy and requires exact
final BRep serialization before comparing successor ordinals. It is same-kernel
lineage evidence, not an independent geometry oracle.

The nine-tool fixture remains refused: its raw result is open, and its unchanged
participation evidence identifies operand 2 as ineffective after normalization.
The explicit eight-tool diagnostic subset does not prove that operand 2 is
redundant or that either result matches the full Framework source.
