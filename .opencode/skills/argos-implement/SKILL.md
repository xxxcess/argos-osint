---
name: argos-implement
description: Implement one architect phase through child coders and tester
---

Follow .opencode/agents/coder.md, AGENTS.md and docs/opencode-v2.md. Send disjoint units to child coder sessions marked EDIT_UNIT, passing .opencode/coder-worker.json model (task override wins). Children format changed files only. After edits finish, tester owns relevant checks, captures and actual image review. Return the report to architect for grounded corrections and phase acceptance. Honor user exclusions. Never substitute direct work for a failed delegation.
