# Local native-review pairing approval handoff

**PR base:** `nightly`, confirmed earlier in this session. **Scope:** isolated macOS native review only. The production pairing API and separate Operator authority remain the trust boundary. **Outcome:** starting a native pairing request automatically opens a local operator confirmation prompt through a private review helper; the operator compares the displayed origin, pairing ID and code, then explicitly approves. Keychain reconnects continue to reuse the retained scoped credential.

| Phase | Intended behavior and evidence | Status |
| --- | --- | --- |
| Implementation | Native Waiting state sends bounded origin, pairing ID, code and expiry to the private local helper once per challenge. The helper shows matching values, requires an explicit click, then uses the existing Operator approval route. Waiting offers Pair again after cancellation or failure. | In progress after user review |
| Static and contract | Validate socket ownership/path, exact loopback origin, bounded messages, no bearer token or polling secret in native handoff, dedup/rate/expiry and explicit confirmation. | Source review pending update; hosted checks pending |
| Feature functionality | CI tests real native Waiting transition and synthetic local helper protocol, invalid input/retry/cancel, and no effect in ordinary production launches or retained Keychain reconnects. | Pending hosted checks |
| Native review | After matching Apple Silicon CI artifact, close the running app, rebuild/open with the local helper, and confirm a pairing request prompts the operator and proceeds only after confirmation. | First live review opened `220a3323` and showed the prompt, but approval did not proceed because the prompt expected typed input. Revised compare-and-click flow needs new CI and native review. |
| Broader gates | Reuse unchanged backend/storage evidence; report any selected CI checks and GUI/Keychain evidence separately. | Pending |

The operator credential remains in a separate private file read by the helper only for a confirmed approval. A native app request alone never grants access. The helper is time bounded, local to the selected review, and has no production deployment role. No implementation build or test runs locally before native review; the task branch is pushed for CI.
