#!/usr/bin/env python3
"""Convert the eval workspace into the skill-creator layout and render transcripts.

<iter>/<name>/<config>/{transcript.jsonl,timing.json,outputs/}  ->
<iter>/eval-<id>-<name>/<config>/run-1/{transcript.md,timing.json,outputs/,eval_metadata.json}
"""
import json, shutil, sys
from pathlib import Path

it = Path(sys.argv[1])

def render(jsonl: Path) -> str:
    out = ["# Transcript", ""]
    for line in jsonl.read_text().splitlines():
        try:
            m = json.loads(line)
        except json.JSONDecodeError:
            continue
        t = m.get("type")
        if t == "assistant":
            for c in m["message"].get("content", []):
                if c.get("type") == "text" and c["text"].strip():
                    out += ["## Assistant", "", c["text"], ""]
                elif c.get("type") == "tool_use":
                    out += [f"### Tool call: {c['name']}", "", "```json", json.dumps(c.get("input", {}))[:3000], "```", ""]
        elif t == "user":
            for c in m["message"].get("content", []) if isinstance(m["message"].get("content"), list) else []:
                if c.get("type") == "tool_result":
                    body = c.get("content")
                    if isinstance(body, list):
                        body = "\n".join(x.get("text", "[image]") if isinstance(x, dict) else str(x) for x in body)
                    out += ["#### Tool result", "", "```", str(body)[:2500], "```", ""]
        elif t == "result":
            out += ["## Final result", "", str(m.get("result", "")), "", f"_turns={m.get('num_turns')} cost_usd={m.get('total_cost_usd')} is_error={m.get('is_error')}_", ""]
    return "\n".join(out)

for meta_path in sorted(it.glob("*/eval_metadata.json")):
    src = meta_path.parent
    if src.name.startswith("eval-"):
        continue
    meta = json.loads(meta_path.read_text())
    dst = it / f"eval-{meta['eval_id']}-{src.name}"
    dst.mkdir(exist_ok=True)
    shutil.copy(meta_path, dst / "eval_metadata.json")
    shutil.copy(src / "prompt.txt", dst / "prompt.txt")
    for config in ("with_skill", "without_skill"):
        cfg_src = src / config
        if not cfg_src.exists():
            continue
        run = dst / config / "run-1"
        # Never discard a run dir that already holds grading: copy only what is missing.
        if run.exists():
            if (run / "grading.json").exists():
                continue
            shutil.rmtree(run)
        shutil.copytree(cfg_src, run)
        shutil.copy(meta_path, run / "eval_metadata.json")
        tj = run / "transcript.jsonl"
        if tj.exists():
            (run / "transcript.md").write_text(render(tj))
        print("prepared", run)
