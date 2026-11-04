#pragma once

#include <string>
#include <vector>

#include <NCollection_IndexedDataMap.hxx>
#include <TopTools_ShapeMapHasher.hxx>
#include <TopoDS_Shape.hxx>
#include <gp_Dir.hxx>
#include <gp_Pnt.hxx>

#include "ops/OpCommon.h"

namespace onecad::ops {

// Deterministic generator-provenance slots for an Extrude tool shape
// (SlotScope::Generator in the topology ledger). Keys are faces and edges of the
// FINAL tool shape; vertices never get a slot.
//
//   face  "side:<entityId>"  wall grown from exactly one profile entity
//         "cap:0|+|-"        planar end face, by signed offset along the extrude
//                            direction from the sketch plane
//   edge  "edge:<a>|<b>"     between two slotted faces, a <= b
//
// Attribution is geometric on the final shape because the prism is reshaped
// (symmetric shift, two-direction fuse + unify, trims, draft), so MakePrism
// history cannot be trusted. A slot claimed by more than one face/edge is
// dropped from every holder: ambiguity means no claim.
using GeneratorSlotMap =
    NCollection_IndexedDataMap<TopoDS_Shape, std::string, TopTools_ShapeMapHasher>;

GeneratorSlotMap attribute_extrude_slots(const TopoDS_Shape& tool,
                                         const std::vector<ProfileEdgeLabel>& labels,
                                         const gp_Pnt& plane_origin, const gp_Dir& extrude_dir,
                                         double tol);

}  // namespace onecad::ops
