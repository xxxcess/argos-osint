---
name: ecc-checkpoint
description: Create a concise manual handoff for Argos work.
---

Use the canonical [agent workflow](../../../docs/wiki/code/agent-workflow.md) with the active host's roles and existing authorization. OpenCode-only restrictions apply when that profile is explicitly selected.

Return under 200 words: latest request, outcome, scope/paths, checks, blockers and one next action. Include commit SHA/type/release impact when relevant. Plans and runtime IDs stay local; follow `docs/wiki/code/planning-artifacts.md` for durable handoffs. Record active process/log ownership and completed-agent status when relevant. A message to a completed agent may not wake it: use the host-supported follow-up/resume/wake operation and verify running status. The handoff plugin saves a bounded automatic handoff at idle/compaction.
