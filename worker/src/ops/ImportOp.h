// ImportOp.h — the `ImportStep` op executor (SCHEMA §7.3, STEP-IMPORT WP-A W1).
//
// An import is an OP on the timeline, not a session verb: full-replay regen has to
// reproduce it, it is fenced like every other step, and it advances
// `historyPrefixHash`. So it runs inside ExecutePlan through the ordinary
// PlanExecutor dispatch, and everything below is shaped by that.
//
// ── Two codecs, one publish path ────────────────────────────────────────────
// `sourceCodec:"step"` replays the pinned v1 reader pipeline (`io::read_step`,
// STEP-IMPORT W0) over the source bytes. `sourceCodec:"brep"` deserializes
// BinTools bytes that the §7.8 `InspectStep` conversion lane produced FROM that
// same pipeline, so replay never re-parses STEP. Both land in the same minting +
// publishing code, which is what makes the two lanes observationally identical
// (same ordinals, same volumes, same geometry signature).
//
// The brep lane must NOT re-sort: the ordinal order was baked into the compound's
// child order at conversion time by `ops::ordered_solids`. Re-deriving it here
// would make the ordinals a function of the CURRENT sort implementation rather
// than of the bytes the document was authored against — a silent re-numbering of
// every imported body on an ordering-policy change. Stored order is the contract.
//
// ── unitScale ───────────────────────────────────────────────────────────────
// Applied at EXECUTE time, in both codecs, as a uniform `gp_Trsf` scale about the
// origin. The persisted brep bytes are canonical and UNSCALED, so editing
// `unitScale` on a brep-codec record re-scales from the canonical geometry
// instead of compounding onto an already-scaled shape.
//
// ── Minting (SCHEMA §2/§7.3, D1) ────────────────────────────────────────────
// N solids ⇒ N `created` events and NO `deleted` parent (creation ex nihilo):
// exactly one solid mints the plain `body_<opId>`, N > 1 mints ordered
// `body_<opId>:<k>` children — the same split-minting rule
// `ops::publish_boolean_result` applies. The element-map delta is empty: the
// bodies are brand new, so no partition entry exists to relabel or remove and
// ElementIds are minted on demand later.
//
// ── Failure model ───────────────────────────────────────────────────────────
// Every user-data problem (missing path, unreadable bytes, wrong codec, unknown
// heal policy, no solids) is a recoverable `OP_FAILED` on the step — NEVER a
// PROTOCOL_ERROR. A hostile file must not tear down the worker.
#pragma once

#include <string>

#include "nlohmann/json.hpp"
#include "ops/OpTypes.h"

namespace onecad::ops {

// Execute one `ImportStep` op into the scratch (SCHEMA §7.3).
OpOutcome execute_import_step(OpContext& ctx, const nlohmann::json& op, const std::string& op_id);

}  // namespace onecad::ops
