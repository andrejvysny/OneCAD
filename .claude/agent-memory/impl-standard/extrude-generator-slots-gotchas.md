---
name: extrude-generator-slots-gotchas
description: Extrude Generator slots - legacy region identity yields no labels; test harness needs regionIdentityVersion 3
metadata:
  type: project
---

FaceBuilder polygon fallback (legacy region identity, ids with `#segN_pN` suffix fail `sketch.getEntity`) records no ProfileEdgeLabel, so slots only appear with `regionIdentityVersion` 2/3. Tests: pass 3 + regionId (parse id from the "available: [..]" refusal of build_profile_face). Boolean Add also lets tool EDGES that survive IsSame inherit slots (rim edges naming a vanished cap).

Rust real-worker tests: comb tests extrude with region "" => legacy => NO slots; get a V3 region id from `comb_region_id` (scratch runtime + finish_sketch) and use `build_comb_fillet_in_region`. Region id hashes entity ids, so re-created ids need the Extrude record's region updated too. Filter lambdas returned from helpers must capture record ids BY VALUE (a `[&param]` capture dangled and silently admitted nothing -> NoCandidates). `measure_fillet_ref` dry-run on a consumed edge reports score 0 / 0 candidates (non-informative).
