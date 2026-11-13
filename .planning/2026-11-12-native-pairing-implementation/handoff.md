# Native pairing — saved checkpoint

[Evidence](acceptance.md) · [Connection contract](../../docs/wiki/code/native-connections.md)

Pairing/Keychain, workstation naming, build progress, workflow docs and diagrams are committed. No user Keychain, app packaging/open, deployment or user-data migration was performed.

Completed: full workspace run, service-host CLI help, required embedded collection and affected retries. The latest affected batch reported1222 passed/1 failed/6 ignored; both stack-overflow targets, bootstrap, log cap and activity ordering passed. Its remaining failure exposed v2 retirement using legacy-folder lookup.

The retirement fix now handles workstation owners, guarded drain/session/admission fences, legacy protection, migrated retiring owners and replay. Formatting and all-target Clippy pass (5m37s). **Runtime remains unverified:** stopped during compilation at the user's request, exit130. Step4 is not declared ready; no final green driver run is claimed.

Resume only the affected required-mode command; retain passing independent evidence:

```sh
CARGO_BUILD_JOBS=2 env -u ARGOS_SURREAL_BIN -u ARGOS_EMBED -u RUST_MIN_STACK \
  ARGOS_SURREAL_MODE=embedded ARGOS_SURREAL_REQUIRED=1 \
  python3 scripts/agent_cargo.py test --workspace --lib --bins --locked \
  --test storage_connect_registry --test storage_classifier_activation \
  --test storage_setup_jobs --test storage_job_actions \
  --test storage_configuration_reset_barrier --test api_contract \
  --no-fail-fast -- --nocapture
```

Library/binary selectors retain the validated workspace feature graph. Preserve current caches, running services and unrelated dist/. Actual GUI/Keychain and hosted platform/mutation evidence remain separate. Schema10 deployment requires compatible bootstrap/gateway/workers; retained deployments were not changed.
