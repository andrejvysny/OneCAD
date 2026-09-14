---
name: worker-build-stale-worktree-paths
description: worker/build CMakeCache/Makefiles can carry a stale absolute path from a renamed/relocated worktree, breaking incremental cmake --build
metadata:
  type: project
---

`worker/build` (Unix Makefiles generator, not Ninja) is gitignored and can be a
leftover build tree configured under a prior worktree path (e.g.
`/Users/andrejvysny/workspace/OneCAD-ai-agent/worker` before the worktree was
renamed to `viewport-hardening`). Symptom: `cmake --build worker/build --target
<x>` fails with "CMakeCache.txt directory ... is different than the directory
... where CMakeCache.txt was created" and "source directory does not exist".

Fix without a full reconfigure/rebuild: `grep -rl "<old-path>" worker/build`
(all matches are text — CMakeCache.txt, CMakeFiles/*.cmake/*.make/*.internal,
compile_commands.json) then `sed -i '' 's#<old-path>#<new-path>#g'` across
those files. Incremental build then proceeds normally, only relinking/rebuilding
what's stale — no full worker rebuild needed. Environment drift, not a code defect.
