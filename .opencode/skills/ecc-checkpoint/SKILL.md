---
name: ecc-checkpoint
description: Create a concise manual handoff for Argos work.
---
Summarize the latest request, outcome, relevant paths, checks, and one next action. Keep it under 200 words. Named task plans stay in `.planning/`; the graphify V2 plugin saves a small automatic handoff at idle and compaction.

When handing off a commit or release task, include the Conventional Commit type/release impact, resulting SHA (if committed), exact scope and pending compatibility/validation work; follow `docs/versioning.md`.

Follow `docs/planning-artifacts.md`: automatic host handoffs and session identifiers stay local. Commit a curated `handoff.md` only when another contributor needs agreed remaining scope/dependencies; name any unresolved local evidence and do not commit raw logs.
