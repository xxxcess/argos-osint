# Native workstation handoff — supporting references

## Start here

This package accompanies the Argos OSINT / Fleetworks native-app redesign.

1. Read [the implementation handoff](argos_workstation_identity_implementation_spec.md) for execution order, verified edit locations, workstation identity migration, credential provisioning, API work and acceptance.
2. Read [the design-system guide](argos_native_design_system_guide.md) for the native control allowlist, screen composition, matte material recipes, brand palette and interaction states.
3. Consult [the original structural reference](argos_osint_workstation_ui_specification.md) for the source layout.
4. Review the seven mockups below for visual composition.
5. Follow the pinned repository links for the relevant implementation and verification rules.

**Precedence:** Latest explicit user decisions govern this package. The current handoff owns execution and API migration; the current guide owns the target UI. The original structural reference's workspace identity, Tauri/Linux-client assumptions, hardware examples, Rust-BERT training and glossy finishes are superseded. The target is the Rune macOS API client, workstation-only identity, matte obsidian black, dim satin graphite, and restrained canonical gold/ivory.

Mockups use synthetic data and illustrate composition. They do not establish API capabilities, exact runtime values, accessibility compliance or completed implementation. Written controls and contracts take precedence over incidental details in an image.

## Package contents

| File | Purpose |
| --- | --- |
| `argos_workstation_identity_implementation_spec.md` | Complete curated execution-agent handoff. |
| `argos_native_design_system_guide.md` | Standing design and native interaction requirements. |
| `argos_osint_workstation_ui_specification.md` | Original user-supplied structural reference, preserved unchanged. |
| `README.md` | This reading order, precedence note and repository-reference index. |
| `mockups/01_workstation_status.png` | Connected workstation identity and backend/resource status. |
| `mockups/02_db_seed_train.png` | Seed/index/training operation cards. |
| `mockups/03_news_cycles.png` | New-cycle action and cycle-history cards. |
| `mockups/04_brain_search.png` | Search results and selected dossier. |
| `mockups/05_osint_tools.png` | Provider/service cards and API-key dialog. |
| `mockups/06_model_assignments.png` | Provider and role tables. |
| `mockups/07_jobs_logs.png` | Job list and selected operational logs. |

Only the latest seven matte concepts are included. The first glossy overview and earlier working audit notes are omitted because they predate the final design decisions.

## Pinned repository documentation

Repository: `xxxcess/argos-osint`  
Branch: `feat/atlas-zero-shot`  
Audited baseline: [`d219d5964a29d50fa99de4cdcf2b228391576b4a`](https://github.com/xxxcess/argos-osint/commit/d219d5964a29d50fa99de4cdcf2b228391576b4a)

All links below are pinned to that commit. Before execution, verify the branch head and inspect the relevant diff if it advanced. The guide and handoff share this same source baseline.

### Read before implementation

| Reference | Why it matters |
| --- | --- |
| [Root AGENTS.md](https://github.com/xxxcess/argos-osint/blob/d219d5964a29d50fa99de4cdcf2b228391576b4a/AGENTS.md) | Current architecture, supported clients, scope and ownership rules. |
| [Native AGENTS.md](https://github.com/xxxcess/argos-osint/blob/d219d5964a29d50fa99de4cdcf2b228391576b4a/apps/fleetworks-desktop/AGENTS.md) | Native app boundaries and Rune-specific workflow. |
| [Native README](https://github.com/xxxcess/argos-osint/blob/d219d5964a29d50fa99de4cdcf2b228391576b4a/apps/fleetworks-desktop/README.md) | Current app capabilities and operation. |
| [Web AGENTS.md](https://github.com/xxxcess/argos-osint/blob/d219d5964a29d50fa99de4cdcf2b228391576b4a/apps/web-desktop/AGENTS.md) | Browser/app boundaries for the limited web changes. |
| [Packages AGENTS.md](https://github.com/xxxcess/argos-osint/blob/d219d5964a29d50fa99de4cdcf2b228391576b4a/packages/AGENTS.md) | Shared React package rules. |
| [Contributor workflow](https://github.com/xxxcess/argos-osint/blob/d219d5964a29d50fa99de4cdcf2b228391576b4a/docs/wiki/code/contributing.md) | Verification authorization, shared caches, isolated browser/backend work and build entry points. |
| [Conventions](https://github.com/xxxcess/argos-osint/blob/d219d5964a29d50fa99de4cdcf2b228391576b4a/docs/wiki/code/conventions.md) | Persistence, async, documentation and diagram conventions. |

### Current contracts and documentation to update

| Reference | How to use it |
| --- | --- |
| [Native connections](https://github.com/xxxcess/argos-osint/blob/d219d5964a29d50fa99de4cdcf2b228391576b4a/docs/wiki/code/native-connections.md) | Existing credentials, context, activity, reconciliation and log contracts. |
| [Native workspace ownership](https://github.com/xxxcess/argos-osint/blob/d219d5964a29d50fa99de4cdcf2b228391576b4a/docs/wiki/code/native-workspaces.md) | **Existing behavior to replace**, including default-child startup and folder lifecycle. It does not override the workstation-only target. |
| [Fleetworks guide](https://github.com/xxxcess/argos-osint/blob/d219d5964a29d50fa99de4cdcf2b228391576b4a/docs/wiki/code/fleetworks.md) | Supported client/service architecture. |
| [Interfaces](https://github.com/xxxcess/argos-osint/blob/d219d5964a29d50fa99de4cdcf2b228391576b4a/docs/wiki/code/interfaces.md) | Shared boundaries affected by the identity migration. |
| [React UI guidance](https://github.com/xxxcess/argos-osint/blob/d219d5964a29d50fa99de4cdcf2b228391576b4a/docs/wiki/code/react-ui.md) | Existing web structure and component conventions; the new matte theme overrides older styling instructions. |
| [Planning artifacts](https://github.com/xxxcess/argos-osint/blob/d219d5964a29d50fa99de4cdcf2b228391576b4a/docs/wiki/code/planning-artifacts.md) | Curated handoff versus local working-plan retention. |

The handoff also identifies the existing `docs/diagrams/workspace`, `workspace-scope` and `workspace-lifecycle` HTML/SVG pairs whose labels and relationships require updates with the new identity contract.

### Verification references

| Reference | Evidence it governs |
| --- | --- |
| [Selected native UI verification](https://github.com/xxxcess/argos-osint/blob/d219d5964a29d50fa99de4cdcf2b228391576b4a/docs/wiki/code/native-verification.md) | Actual Rune Runtime, PNG/tree/script captures and separate installed-app checks. |
| [Testing guidance](https://github.com/xxxcess/argos-osint/blob/d219d5964a29d50fa99de4cdcf2b228391576b4a/.opencode/skills/argos-testing/SKILL.md) | Meaningful offline fixtures, isolation, permissions, scope and honest pass/fail/skip reporting. |
| [Rune verification skill](https://github.com/xxxcess/argos-osint/blob/d219d5964a29d50fa99de4cdcf2b228391576b4a/.opencode/skills/fleetworks-rune-verify/SKILL.md) | Required native view/handler workflow. |
| [CI guide](https://github.com/xxxcess/argos-osint/blob/d219d5964a29d50fa99de4cdcf2b228391576b4a/docs/wiki/code/ci.md) | Existing scoped checks and both Mac architecture artifact lanes. |
| [React UI testing](https://github.com/xxxcess/argos-osint/blob/d219d5964a29d50fa99de4cdcf2b228391576b4a/docs/wiki/code/react-ui-testing.md) | Existing frontend fixtures and browser evidence. |

## Key source anchors

The handoff contains the complete edit map. These anchors are useful when reviewing its central decisions:

- [Native shell and state](https://github.com/xxxcess/argos-osint/blob/d219d5964a29d50fa99de4cdcf2b228391576b4a/apps/fleetworks-desktop/src/app.rs)
- [Native view composition](https://github.com/xxxcess/argos-osint/blob/d219d5964a29d50fa99de4cdcf2b228391576b4a/apps/fleetworks-desktop/src/views.rs)
- [Connection contracts](https://github.com/xxxcess/argos-osint/blob/d219d5964a29d50fa99de4cdcf2b228391576b4a/crates/fleetworks-contracts/src/connect.rs)
- [Workstation registry](https://github.com/xxxcess/argos-osint/blob/d219d5964a29d50fa99de4cdcf2b228391576b4a/crates/argos-osint-core/src/storage/connect_registry.rs)
- [Existing read-only request reconciliation](https://github.com/xxxcess/argos-osint/blob/d219d5964a29d50fa99de4cdcf2b228391576b4a/crates/argos-osint-api/src/request_reconciliation.rs)
- [Browser Connect sessions](https://github.com/xxxcess/argos-osint/blob/d219d5964a29d50fa99de4cdcf2b228391576b4a/apps/web-desktop/connect-session.mjs)
- [Native material recipes](https://github.com/xxxcess/argos-osint/blob/d219d5964a29d50fa99de4cdcf2b228391576b4a/apps/fleetworks-desktop/src/material.rs)
- [Canonical five-ray logo](https://github.com/xxxcess/argos-osint/blob/d219d5964a29d50fa99de4cdcf2b228391576b4a/assets/brand/fleetworks-mark.svg)

This bundle contains specifications and illustrative assets. Product implementation and runtime validation remain the execution agent's work.

