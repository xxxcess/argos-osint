# Set up Embedded Inbox Assistant

Inbox Assistant starts Off. In an Embedded workstation, turn on **Backend inference**. The **Assistant** tab in Workstation Settings shows the saved preferences, effective engines, startup progress and current generator context usage. Open Assistant from Workstation Status or use the Inbox composer. Sending becomes available when the selected generator is ready. The composer shows the reason whenever sending is unavailable.

- **Generator** offers **Automatic**, **FLAN-T5 Small** (~308 MB), **FLAN-T5 Base** (~990 MB), **GPT-2 Small** (~548 MB) and **GPT-2 Medium** (~1.52 GB). Automatic uses fresh headroom and verified assets to choose between the FLAN-T5 models. Each explicit choice stays selected if it cannot fit or provision, with the cause shown in Settings.
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

The default pack carries the verified worker, its runtime libraries and FLAN-T5 Small. With Automatic selected, the companion prefers FLAN-T5 Base when measured memory headroom allows it. When Base assets cannot be provisioned, the verified Small checkpoint in the offline pack is available as a fallback. With a GPT-2 choice, a compatible worker uses a verified named pack or provisions the pinned checkpoint and BPE files into the private profile. Downloads are used to obtain model files. Response generation runs inside the existing offline worker sandbox. The companion loads one model at a time. The [news classifier](../docs/wiki/code/classifier.md) uses its own model and configuration.

## Host requirements

The worker requires Linux Landlock ABI 3 or newer, or macOS Seatbelt. The companion starts it with read access to the verified pack, selected model assets, and system loader. Workstation Settings and Inbox show whether the pack is missing or invalid, the sandbox is unavailable, or the worker has insufficient resources or cannot start.

At startup, the companion reserves 1 GiB of available memory for the workstation and its other workloads. The worker’s resident-memory limits are 3,584 MiB for FLAN-T5 Base, 1,536 MiB for FLAN-T5 Small, 2,048 MiB for GPT-2 Small and 4,608 MiB for GPT-2 Medium. If available memory cannot accommodate the chosen worker and the reserve, Settings reports insufficient resources. Model asset downloads also leave a 512 MiB reserve on the private profile filesystem. The fixture and operator ceiling `FLEETWORKS_QA_MEMORY_MIB` accepts 512 to 6144 MiB. The worker runs at reduced priority with one CPU thread by default. `FLEETWORKS_QA_CPU_THREADS` accepts 1 to 4.

The offline package smoke uses a 1,536 MiB ceiling and verifies Small without network access. The Linux pack job exercises Base separately. Turning inference off cancels active work, unloads workers, and leaves the preferences and Inbox notifications available.

## Workstation question benchmark

Run the workstation questions through a packaged Embedded companion with private state, home and temporary directories. The benchmark checks the bundle target and companion SHA-256 against the app manifest before launch. It checks that **Generator → Automatic** and **Decision → Rules only** are the defaults, then enables Backend inference with those preferences. Use the Small pack directory and manifest SHA-256 from a verified installation or artifact receipt.

```sh
mkdir -p .agent-scratch
python3 -B scripts/fleetworks_workstation_bench.py \
  --app "$FLEETWORKS_APP" --pack-dir "$QA_PACK_DIR" \
  --manifest-sha256 "$QA_MANIFEST_SHA256" > .agent-scratch/workstation-bench.json
```

Add `--base-model-assets "$PINNED_BASE_MODEL_DIR"` if you have the verified Base files cached and want Automatic to choose with the host's actual memory headroom. With only the Small pack present, the benchmark caps generator memory at 1,536 MiB to stay offline. Each run creates a synthetic registration, checks that a Rules only lookup resolves the catalog's current status without creating an exchange, and reads readiness and job counts over private IPC. Resource checks use the frozen samples cited in each turn.

The JSON records answer text, citations, measured model use, and admission, first-draft and terminal latency per question. Post-admission latency includes execution and IPC polling. On Linux the report also records peak resident worker memory observed from the kernel. The benchmark verifies complete cited passages, checks conversational text for repeated sentences, and flags echoed prompt labels.

Use `--warmup-turns` for unmeasured greetings and `--iterations` to repeat the measured cases with a new isolated registration each time. A completed run reports case failures in JSON. Use `--fail-on-error` to also exit nonzero on a fact, route, or reply-style check failure. Launch, setup, and transport errors always exit nonzero.

[Assistant contract](../docs/wiki/code/assistant-chat-contract.md) · [Hosted verification](../docs/wiki/code/ci.md)
