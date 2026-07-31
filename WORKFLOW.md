---
tracker:
  kind: file
  data_root: ~/.symphony
  project_id: onecad
workspace:
  mode: single_dir
  repo: /Users/andrejvysny/workspace/CAD/OneCAD-Tauri
agent:
  backend: claude-sdk
  permission_mode: bypassPermissions
  max_concurrent_agents: 5
  max_concurrent_plans: 3
server:
  port: 4500
projects:
  - name: OneCAD
    project_id: onecad
    repo: /Users/andrejvysny/workspace/CAD/OneCAD-Tauri
    identifier: ONECAD
    config_dir: ~/.claude
    dev_server:
      start: npm run tauri dev
---

You have been assigned issue {{ issue.identifier }}: "{{ issue.title }}".

<issue>
Identifier: {{ issue.identifier }}
Issue id (pass as task_id to the tracker tools): {{ issue.id }}
Title: {{ issue.title }}
{% if issue.priority %}Priority: {{ issue.priority }}
{% endif %}{% if issue.labels.size > 0 %}Labels: {{ issue.labels | join: ", " }}
{% endif %}Description:
{% if issue.description %}{{ issue.description }}{% else %}(No description was provided. Treat the title as the specification; if it is too vague to implement safely, follow the blocked protocol.){% endif %}
</issue>

Implement this issue end to end: read it with tracker_get_task, move it to "In Progress" with a short plan comment, make the change, confirm the project's build/tests/lint pass, commit locally, then post an evidence-backed summary comment (with the verification output and commit SHA) and move the issue to "Human Review". Keep every change scoped to this issue.
