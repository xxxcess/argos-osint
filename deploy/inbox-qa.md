# Set up Embedded Inbox Assistant

Inbox Assistant starts Off. In an Embedded workstation, turn on **Backend inference**. The **Assistant** tab in Workstation Settings shows the automatically selected model, its weight size, startup progress and the latest context usage. Open Assistant from Workstation Status or use the Inbox composer once the worker is ready. Workstation Status keeps its deterministic warnings and diagnostic checks available during startup and when inference is off.

## Verified offline packs

Use the offline pack and digest record for the app’s architecture. Obtain the digests from the trusted release or the exact CI artifact used for verification. The importer verifies the archive, manifest, native architecture, worker, libraries, and pinned model assets.

```sh
python3 scripts/fleetworks_qa_pack.py install \
  --target "$TARGET" --archive "$QA_ARCHIVE" \
  --archive-sha256 "$QA_ARCHIVE_SHA256" --manifest-sha256 "$QA_MANIFEST_SHA256" \
  --output "$PRIVATE_PACK_PARENT/$PACK_VERSION" --state-dir "$EMBEDDED_STATE_DIR"
```

Use the Embedded companion’s state directory for the selected installation. The pack parent and the private state directory must belong to the current user. To replace a pack, turn the Assistant Off, import into a new version directory, then enable it.

The default pack carries the verified worker, its runtime libraries and FLAN-T5 Small. Install it once on a fresh workstation. The companion prefers FLAN-T5 Base when measured memory headroom allows it. It reuses a verified cached Base checkpoint or provisions its pinned files in the private profile. When Base assets cannot be provisioned, the verified Small checkpoint in the offline pack is available as a fallback. The companion loads one model at a time. The [news classifier](../docs/wiki/code/classifier.md) uses its own model and configuration.

## Host requirements

The worker requires Linux Landlock ABI 3 or newer, or macOS Seatbelt. The companion starts it with read access to the verified pack, selected model assets, and system loader. Workstation Settings and Inbox show whether the pack is missing or invalid, the sandbox is unavailable, or the worker has insufficient resources or cannot start.

At startup, the companion reserves 1 GiB of available memory for the workstation and its other workloads. The worker’s resident-memory limits are 3,584 MiB for Base and 1,536 MiB for Small. If available memory cannot accommodate Small and the reserve, Settings reports insufficient resources. Model asset downloads also leave a 512 MiB reserve on the private profile filesystem. The fixture and operator ceiling `FLEETWORKS_QA_MEMORY_MIB` accepts 512 to 6144 MiB. The worker runs at reduced priority with one CPU thread by default. `FLEETWORKS_QA_CPU_THREADS` accepts 1 to 4.

The offline package smoke uses a 1,536 MiB ceiling and verifies Small without network access. The Linux pack job exercises Base separately. Turning inference off cancels active work, unloads the model and leaves Inbox notifications available.

[Assistant contract](../docs/wiki/code/assistant-chat-contract.md) · [Hosted verification](../docs/wiki/code/ci.md)
