---
name: jevx
description: Suggest the most relevant Codex Skill for the current task using jevx and Jev. Use when the task may benefit from a project or user Skill, especially for documents, code review, testing, UI, research, or repository workflows.
---

# jevx

Use `jevx` as an advisory Skill selector. It does not load, execute, or modify a Skill automatically.

## Suggest a Skill

Pass only the current user request to jevx. Do not send the full conversation or Skill body.

```bash
jevx skills suggest --prompt "<current user request>" --json
```

If the result contains `decision: "selected"`, show the selected Skill and its probability. The agent may then decide whether the Skill is relevant under the normal Codex rules. If the result is `none`, continue without loading a Skill.

`jevx` runs in shadow mode in v1. Never treat its output as permission to execute commands or bypass repository instructions.
