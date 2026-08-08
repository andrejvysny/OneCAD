# 0007 — Contribution order is explicit, never load order

- Status: Accepted
- Date: 2026-08-08

## Context

Today's toolbar, shortcut tables, inspector sections and editor mount order are
static arrays: the order is whatever the source says, and it is reviewable.

Replacing them with registries introduces a failure mode that static arrays do not
have — order becoming a side effect of when each module happened to register.
That is invisible in review, unstable under lazy loading, and produces UI that
differs between runs.

Editor mount order specifically is load-bearing: the overlays are absolutely
positioned siblings, so DOM order decides stacking inside a z-index band. A past
defect had tool chips render under the side panels and become unclickable.

## Decision

Every contribution declares its placement explicitly:

```
group    = "modeling.modify"
priority = 200
```

Iteration order is `(priority, insertion index)`, where insertion index is only a
last-resort tiebreak for two contributions that declared the same priority. The
same set of contributions must produce the same UI order on every run,
regardless of registration order.

**`group` is NOT a sort key** — it is consumer metadata, and the consumer decides
what to do with it (the toolbar draws a separator wherever the group changes
between two adjacent entries). Sorting by group first would order the UI by the
group's NAME, alphabetically, which is never the intended visual order:
`modeling.create` would precede `modeling.solid` by accident of spelling rather
than by intent. A group is therefore expected to be a contiguous run of
priorities, and it is the priorities that put it there.

Sections are ordered by category: owner-specific primary sections, then platform
sections, then third-party sections.

This is verified, not asserted: `src/test/contracts/` holds frozen copies of the
shipped toolbar, keyboard bindings, editor mount order and inspector section
order. The probes that read the production side may change as the mechanism
changes; the frozen contracts do not move without a recorded product decision.

## Cost

Contributors must pick a group and a priority instead of relying on file order,
and priority numbers need occasional renumbering. Cheap next to a UI that
rearranges itself when a module's load timing changes.
