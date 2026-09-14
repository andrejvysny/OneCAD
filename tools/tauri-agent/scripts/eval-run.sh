#!/bin/bash
# Run one skill-creator eval prompt through headless Claude Code with the tauri-agent MCP server.
#   eval-run.sh <eval-name> <with_skill|without_skill> <prompt-file> <outdir>
# with_skill: cwd = repo (project + global skills visible). without_skill: cwd = scratch dir,
# --setting-sources project (no user or project skills), absolute MCP config, so the same tools exist but
# no skill text does. Runs are sequential by construction (one WebDriver port).
set -u
NAME="$1"; VARIANT="$2"; PROMPT_FILE="$3"; OUT="$4"
REPO=/Users/andrejvysny/workspace/OneCAD
mkdir -p "$OUT/outputs"
PROMPT=$(cat "$PROMPT_FILE")
TOOLS='mcp__tauri-agent__*,Read,Glob,Grep,Bash(pgrep:*),Bash(ls:*),Bash(lsof:*),Bash(cat:*),Bash(bun:*),Bash(bunx:*),Bash(cargo:*),Bash(node:*),Bash(curl:*),Bash(open:*),Bash(screencapture:*),Bash(osascript:*),Bash(cliclick:*),Bash(sleep:*),Bash(echo:*)'
COMMON=(--print --output-format stream-json --verbose --max-turns 150 --allowedTools "$TOOLS" --strict-mcp-config)
START=$(date +%s)
if [ "$VARIANT" = "with_skill" ]; then
  ( cd "$REPO" && claude "$PROMPT" "${COMMON[@]}" --mcp-config "$REPO/.mcp.json" ) > "$OUT/transcript.jsonl" 2> "$OUT/stderr.log"
else
  SCRATCH=$(mktemp -d /tmp/tauri-agent-baseline.XXXX)
  cat > "$SCRATCH/mcp.json" <<JSON
{"mcpServers":{"tauri-agent":{"type":"stdio","command":"/Users/andrejvysny/.bun/bin/bun","args":["$REPO/tools/tauri-agent/src/mcp/server.ts"],"env":{"TAURI_AGENT_ROOT":"$REPO","TAURI_AGENT_LOG":"info"}}}}
JSON
  # --setting-sources project from a scratch cwd and NO --add-dir: --add-dir would expose the repo's
  # .claude/skills (measured: the overlay skill loaded), so the baseline must not see the repo at all.
  ( cd "$SCRATCH" && claude "$PROMPT" "${COMMON[@]}" --setting-sources project --mcp-config "$SCRATCH/mcp.json" ) > "$OUT/transcript.jsonl" 2> "$OUT/stderr.log"
fi
RC=$?
END=$(date +%s)
python3 - "$OUT" "$START" "$END" "$RC" <<'PY'
import json,sys,glob,os,shutil
out,start,end,rc=sys.argv[1],int(sys.argv[2]),int(sys.argv[3]),int(sys.argv[4])
result=None; tokens=0; cost=None; turns=None
for line in open(f"{out}/transcript.jsonl"):
    try: m=json.loads(line)
    except: continue
    if m.get("type")=="result":
        result=m; cost=m.get("total_cost_usd"); turns=m.get("num_turns")
        u=m.get("usage") or {}
        tokens=sum(int(u.get(k,0) or 0) for k in ("input_tokens","output_tokens","cache_creation_input_tokens","cache_read_input_tokens"))
open(f"{out}/outputs/report.md","w").write((result or {}).get("result","") if result else "(no result message)")
json.dump({"total_tokens":tokens,"duration_ms":(end-start)*1000,"total_duration_seconds":end-start,"cost_usd":cost,"num_turns":turns,"exit_code":rc},open(f"{out}/timing.json","w"),indent=2)
# copy every artifact session created during this run
root="/Users/andrejvysny/workspace/OneCAD/.tauri-agent/artifacts"
for d in sorted(glob.glob(f"{root}/s-*")):
    if os.path.getmtime(d) >= start-2:
        dst=f"{out}/outputs/{os.path.basename(d)}"
        if not os.path.exists(dst): shutil.copytree(d,dst)
print("done", out, "rc", rc, "turns", turns, "cost", cost, "seconds", end-start)
PY
# never leave the app running between evals
pgrep -f 'target/debug/onecad|onecad-worker-' >/dev/null && { echo "SURVIVORS after $NAME/$VARIANT:"; pgrep -fl 'target/debug/onecad|onecad-worker-'; pkill -TERM -f 'workspace/OneCAD/src-tauri/target/debug/onecad'; sleep 2; pkill -KILL -f 'workspace/OneCAD/src-tauri/target/debug/onecad'; pkill -TERM -f 'workspace/OneCAD/node_modules/.bin/vite'; pkill -TERM -f 'workspace/OneCAD/node_modules/.bin/tauri dev'; }
exit 0
