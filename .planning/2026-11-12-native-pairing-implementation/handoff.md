# Native pairing implementation — handoff

[Actual verification](acceptance.md) · [Connection contract](../../docs/wiki/code/native-connections.md#pairing-and-keychain-agreed-target)

Implemented: public bounded pairing start/poll; separate Operator approve/deny/revoke; durable scope/installation/epoch/grant fences; Keychain-backed native setup/reconnect/recovery; exact-retry journals; explicit operator file mode; fresh review Operator provisioning. Root owns integration/Cargo and commits. Dependency foundation committed `6eef4cc`; feature source is frozen for checks except demonstrated blockers.

Use `python3 scripts/fleetworks_pairing.py --origin <gateway> --operator-token-file <private-operator-file> approve --pairing-id <displayed-id> --approval-code <displayed-code>` from a separately trusted operator terminal. Native users enter a URL and wait for approval. Existing reviews require explicit independent Operator provisioning; initialization never edits retained deployments.

Schema 10 requires coordinated compatible bootstrap/gateway/workers. No automatic bootstrap, container replacement, user-Keychain write or data migration was performed.

Completed: 146 Rust unit/native/SDK/catalog tests; 15 Python operator/provisioning cases; backend server-feature production check; locked Linux runtime/search dependency resolution; selected 12 native PNG/tree inspections and 5 document figures.

Current verification: formatter, all-target Clippy and full workspace passed; actual service-host CLI help passed. Required collection exposed seven targets; affected retries cleared six, including both production stack-overflow paths. Latest affected batch:1222 passed/1 failed/6 ignored. The sole remaining failure is public v2 retirement returning NotFound through legacy-folder lookup; a bounded production repair plus migrated-retiring compatibility regression are under review. No overall gate pass is claimed. Read acceptance for commands, counts and retained evidence. Native app packaging/open, real Keychain and hosted platforms remain separate; no user data or running stack was changed.
