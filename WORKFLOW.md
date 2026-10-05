# Agent task workflow

Use the private [ONECAD project](https://app.plane.so/andrejvysny/projects/b7ff7739-7b47-449c-93e7-81f8d74aeaee/issues/) as the task tracker. Read the assigned Task, its parent, dependencies, acceptance criteria and relevant canonical Pages before implementation. Retain code-coupled contracts in Git and consult [Documentation index](https://app.plane.so/andrejvysny/projects/b7ff7739-7b47-449c-93e7-81f8d74aeaee/pages/0bfe867d-215a-4cfb-a501-9f628976bc9d).

The former Symphony file-tracker configuration used an obsolete checkout path and npm command. It is preserved as historical provenance at [a6d8a2da](https://github.com/andrejvysny/OneCAD/blob/a6d8a2dacad7b1759cd22d0f3eb64992b9c411b0/WORKFLOW.md); it is not active project-management configuration. Do not recreate a local tracker or invent an unsupported Plane adapter.

Implement only the authorized Task scope. Preserve unrelated work. Use Bun for this repository. Reproduce defects before fixing them; run proportionate checks and retain exact results. Do not claim native acceptance from mock browser tests.

Record approved work and validation in the existing Task or canonical Page. Propose future work as Tasks and reusable knowledge in the global Wiki; obtain approval before new saves unless already authorized. Never auto-commit, push, pull or mark incomplete acceptance complete.

Unresolved questions: none for this workflow. A live Symphony-to-Plane adapter remains outside this documentation migration and requires an explicit implementation Task if that runner is used.
