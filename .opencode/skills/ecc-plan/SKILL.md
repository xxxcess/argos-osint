---
name: ecc-plan
description: Plan a bounded Argos implementation from graph and source evidence.
---
1. Run `graphify query` for the change area, then inspect returned files and callers.
2. Identify the smallest coherent change, acceptance checks, and risks.
3. Return ordered steps with paths. Use `argos-plan` for multi-step task state; simple tasks need no milestone or phase ceremony.

For new or significant architectural/conceptual shifts, include affected doc/figure paths, owners and consistency checks using `docs/conventions.md#architecture-and-concepts`.
