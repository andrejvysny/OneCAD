#!/usr/bin/env python3
"""Mechanical facts per eval run, for the grader: journal-derived evidence and transcript hygiene."""
import json, re, sys, glob
from pathlib import Path

it = Path(sys.argv[1])
for run in sorted(it.glob("eval-*/*/run-1")):
    facts = {"run": str(run)}
    tr = run / "transcript.jsonl"
    tool_calls, texts, debug_calls, shell_cmds = [], [], [], []
    for line in tr.read_text().splitlines():
        try:
            m = json.loads(line)
        except json.JSONDecodeError:
            continue
        if m.get("type") == "assistant":
            for c in m["message"].get("content", []):
                if c.get("type") == "tool_use":
                    n = c["name"].replace("mcp__tauri-agent__", "")
                    tool_calls.append(n)
                    if n.startswith("debug_"):
                        debug_calls.append(c.get("input"))
                    if n == "Bash":
                        shell_cmds.append(str(c["input"].get("command", "")))
                elif c.get("type") == "text":
                    texts.append(c["text"])
        elif m.get("type") == "result":
            facts["result_subtype"] = m.get("subtype")
            facts["num_turns"] = m.get("num_turns")
    report = (run / "outputs" / "report.md").read_text() if (run / "outputs" / "report.md").exists() else ""
    facts["tool_counts"] = {k: tool_calls.count(k) for k in sorted(set(tool_calls))}
    facts["debug_calls"] = len(debug_calls)
    facts["report_action_ids"] = sorted(set(re.findall(r"\bA-\d{3}\b", report)))
    # A cited actionId that maps to a debug_* tool is a diagnostic used as evidence.
    debug_ids = set()
    for s in glob.glob(str(run / "outputs" / "s-*" / "journal.jsonl")):
        for l in Path(s).read_text().splitlines():
            if l.strip():
                e = json.loads(l)
                if e["tool"].startswith("debug_"):
                    debug_ids.add(e["actionId"])
    cited_debug = sorted(debug_ids & set(facts["report_action_ids"]))
    facts["report_cites_debug"] = bool(cited_debug) or bool(re.search(r"debug_eval_js|debug_invoke", report))
    facts["report_cited_debug_action_ids"] = cited_debug
    facts["report_mentions_mode"] = bool(re.search(r"real_user|cgevent|native", report))
    alltext = "\n".join(texts) + "\n" + "\n".join(shell_cmds)
    facts["playwright_or_devurl"] = bool(re.search(r"bunx playwright|playwright test|page\.goto|http://localhost:1420|http://localhost:4177|chromium", alltext, re.I))
    facts["shell_cmds"] = shell_cmds[:12]
    facts["skill_invoked"] = tool_calls.count("Skill")
    sessions = sorted(glob.glob(str(run / "outputs" / "s-*")))
    facts["sessions"] = []
    for s in sessions:
        jf = Path(s) / "journal.jsonl"
        entries = [json.loads(l) for l in jf.read_text().splitlines() if l.strip()] if jf.exists() else []
        tools = [e["tool"] for e in entries]
        cg = [e for e in entries if (e["result"].get("backend") == "cgevent" and e["result"].get("mode") == "real_user")]
        drags = [e for e in entries if e["tool"] == "pointer_drag_path"]
        facts["sessions"].append({
            "session": Path(s).name,
            "entries": len(entries),
            "first_tool": tools[0] if tools else None,
            "last_tool": tools[-1] if tools else None,
            "cgevent_actions": len(cg),
            "cgevent_tools": sorted(set(e["tool"] for e in cg)),
            "hover_with_shot": sum(1 for e in cg if e["tool"] == "pointer_hover" and e["result"].get("screenshot")),
            "window_shots": sum(1 for e in entries if (e["result"].get("screenshot") or {}).get("captureMode") == "window"),
            "png_files": len(list(Path(s).glob("*.png"))),
            "drag_paths": [{"button": e["input"].get("button"), "mods": e["input"].get("mods"), "status": e["result"].get("status")} for e in drags],
            "scrolls": sum(1 for e in entries if e["tool"] == "pointer_scroll"),
            "left_drag_in_viewport": any(e["tool"] in ("pointer_drag", "pointer_drag_path") and (e["input"].get("button") in (None, "left")) for e in entries),
            "keypress_e_with_sketch_warning": [e["result"].get("warnings") for e in entries if e["tool"] == "keyboard_press" and str(e["input"].get("key")).lower() == "e"],
            "wait_for_history_or_extrude": any(e["tool"] == "wait_for" and re.search(r"history|Extrude", json.dumps(e["input"])) for e in entries),
            "observe_logs_regen": any(e["tool"] == "observe_logs" and "regen" in json.dumps(e["input"]) for e in entries),
            "stop_ok": any(e["tool"] == "session_stop" and e["result"].get("status") == "ok" for e in entries),
            "errors": [(e["actionId"], e["tool"], (e["result"].get("error") or {}).get("code")) for e in entries if e["result"].get("status") == "error"],
        })
    (run / "facts.json").write_text(json.dumps(facts, indent=2))
    print(run.parent.parent.name, run.parent.name, json.dumps({k: facts.get(k) for k in ("num_turns", "debug_calls", "report_cites_debug", "playwright_or_devurl", "skill_invoked")}), [ (s["cgevent_actions"], s["first_tool"], s["last_tool"], s["window_shots"], s["stop_ok"]) for s in facts["sessions"]])
