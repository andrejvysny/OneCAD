# Decisions

## 2026-02-09 Plan Start
- Guide-first always: when guide candidates exist, they override the findBestSnap winner
- Multi-guide tie-break: nearest guide by distance
- Do NOT modify SnapManager::findBestSnap ranking logic
- Infinite guides: viewport-clipped ray projection, not infinite geometry
