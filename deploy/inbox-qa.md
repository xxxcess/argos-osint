# Set up Embedded Inbox Assistant

Inbox Assistant starts Off. In an Embedded workstation, turn on **Backend inference** and use the **Assistant** section in Workstation Settings to choose a model. Settings show the model’s weight download size, provisioning progress, and readiness. The Inbox composer becomes available when the selected model is ready.

## Verified offline packs

Use the offline pack and digest record for the app’s architecture. Obtain the digests from the trusted release or the exact CI artifact used for verification. The importer verifies the archive, manifest, native architecture, worker, libraries, and pinned model assets.

```sh
python3 scripts/fleetworks_qa_pack.py install \
  --target "$TARGET" --archive "$QA_ARCHIVE" \
  --archive-sha256 "$QA_ARCHIVE_SHA256" --manifest-sha256 "$QA_MANIFEST_SHA256" \
  --output "$PRIVATE_PACK_PARENT/$PACK_VERSION" --state-dir "$EMBEDDED_STATE_DIR"
```

Use the Embedded companion’s state directory for the selected installation. The pack parent and the private state directory must belong to the current user. To replace a pack, turn the Assistant Off, import into a new version directory, then enable it.

The default pack carries the verified worker, its runtime libraries and FLAN-T5 Small. Install it once on a fresh workstation. Selecting FLAN-T5 Base, GPT-2 Small, or GPT-2 Medium provisions the chosen model’s pinned assets in the workstation profile. The companion loads one model at a time. The [news classifier](../docs/wiki/code/classifier.md) uses its own model and configuration.

## Host requirements

The worker requires Linux Landlock ABI 3 or newer, or macOS Seatbelt. The companion starts it with read access to the verified pack, selected model assets, and system loader. Workstation Settings and Inbox show whether the pack is missing or invalid, the sandbox is unavailable, or the worker cannot start.

The companion accepts `FLEETWORKS_QA_CPU_THREADS` from 1 to 4 and `FLEETWORKS_QA_MEMORY_MIB` from 512 to 6144. It uses one CPU thread by default and applies a resident-memory limit of 1,536 MiB for FLAN-T5 Small, 3,584 MiB for FLAN-T5 Base, 2,048 MiB for GPT-2 Small, and 4,608 MiB for GPT-2 Medium. The offline package smoke verifies the default pack with networking disabled.

[Assistant contract](../docs/wiki/code/assistant-chat-contract.md) · [Hosted verification](../docs/wiki/code/ci.md)
