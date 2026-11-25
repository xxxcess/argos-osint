# Set up Embedded Inbox Assistant

Inbox Assistant starts Off. In an Embedded workstation, turn on **Backend inference**. The **Assistant** tab in Workstation Settings shows the saved preferences, effective engines, startup progress and current generator context usage. Open Assistant from Workstation Status or use the Inbox composer. Direct questions and explicit links are available while the generator prepares.

- **Generator** offers **Automatic**, **FLAN-T5 Small** (~308 MB) and **FLAN-T5 Base** (~990 MB). Automatic uses fresh headroom and verified assets to choose a worker. Explicit Small always requests Small. If explicit Base cannot fit or provision, it stays selected and the panel displays the cause.
- **Decision** offers **Rules only**, **Julia-1 Q8** (168 MB) and **vLLM d3-edge Q8** (806 MB). The companion saves either model preference with **Rules only** effective until that exact model and runtime qualify on the current native target. Saving an unqualified choice triggers no downloads. A Decision save keeps the current generator warm.

Workstation Status keeps its deterministic warnings and diagnostic checks available during startup and when inference is off.

## Verified offline packs

Use the offline pack and digest record for the app’s architecture. Obtain the digests from the trusted release or the exact CI artifact used for verification. The importer verifies the archive, manifest, native architecture, worker, libraries, and pinned model assets.

```sh
python3 scripts/fleetworks_qa_pack.py install \
  --target "$TARGET" --archive "$QA_ARCHIVE" \
  --archive-sha256 "$QA_ARCHIVE_SHA256" --manifest-sha256 "$QA_MANIFEST_SHA256" \
  --output "$PRIVATE_PACK_PARENT/$PACK_VERSION" --state-dir "$EMBEDDED_STATE_DIR"
```

Use the Embedded companion’s state directory for the selected installation. The pack parent and the private state directory must belong to the current user. To replace a pack, turn the Assistant Off, import into a new version directory, then enable it.

The default pack carries the verified worker, its runtime libraries and FLAN-T5 Small. Install it once on a fresh workstation. With Automatic selected, the companion prefers FLAN-T5 Base when measured memory headroom allows it. It reuses a verified cached Base checkpoint or provisions its pinned files in the private profile. When Base assets cannot be provisioned, the verified Small checkpoint in the offline pack is available as a fallback. The companion loads one model at a time. The [news classifier](../docs/wiki/code/classifier.md) uses its own model and configuration.

## Host requirements

The worker requires Linux Landlock ABI 3 or newer, or macOS Seatbelt. The companion starts it with read access to the verified pack, selected model assets, and system loader. Workstation Settings and Inbox show whether the pack is missing or invalid, the sandbox is unavailable, or the worker has insufficient resources or cannot start.

At startup, the companion reserves 1 GiB of available memory for the workstation and its other workloads. The worker’s resident-memory limits are 3,584 MiB for Base and 1,536 MiB for Small. If available memory cannot accommodate Small and the reserve, Settings reports insufficient resources. Model asset downloads also leave a 512 MiB reserve on the private profile filesystem. The fixture and operator ceiling `FLEETWORKS_QA_MEMORY_MIB` accepts 512 to 6144 MiB. The worker runs at reduced priority with one CPU thread by default. `FLEETWORKS_QA_CPU_THREADS` accepts 1 to 4.

The offline package smoke uses a 1,536 MiB ceiling and verifies Small without network access. The Linux pack job exercises Base separately. Turning inference off cancels active work, unloads workers, and leaves the preferences and Inbox notifications available.

## Workstation question benchmark

Run the workstation questions with **Generator → Automatic** and **Decision → Rules only** through a packaged Embedded companion in a fresh private state directory. Use the Small pack directory and manifest SHA-256 from a verified installation or artifact receipt.

```sh
mkdir -p .agent-scratch
python3 -B scripts/fleetworks_workstation_bench.py \
  --app "$FLEETWORKS_APP" --pack-dir "$QA_PACK_DIR" \
  --manifest-sha256 "$QA_MANIFEST_SHA256" > .agent-scratch/workstation-bench.json
```

Add `--base-model-assets "$PINNED_BASE_MODEL_DIR"` if you have the verified Base files cached and want Automatic to choose with the host's actual memory headroom. With only the Small pack present, the benchmark caps generator memory at 1,536 MiB to stay offline. Each run creates a synthetic registration, checks that a Rules only lookup resolves the catalog's current status without creating an exchange, and reads readiness and job counts over private IPC. Resource checks use the frozen samples cited in each turn.

The JSON records answer text, citations, expected and observed response paths, and first-draft and terminal latency per question. It also flags repeated sentences and echoed prompt labels. Read factual checks and generated-reply counts together. Exact facts can pass with zero model input, and a turn with model input can still finish using verified evidence. A completed run reports quality failures in its cases and summary. A launch, setup, or transport failure exits with an error.

[Assistant contract](../docs/wiki/code/assistant-chat-contract.md) · [Hosted verification](../docs/wiki/code/ci.md)
