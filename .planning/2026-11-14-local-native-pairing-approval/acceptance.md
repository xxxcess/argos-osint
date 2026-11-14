# Local native-review pairing approval handoff

**PR base:** `nightly`, confirmed earlier in this session. **Scope:** isolated macOS native review only. The production pairing API and separate Operator authority remain the trust boundary. **Outcome:** starting a native pairing request automatically opens a local operator confirmation prompt through a private review helper; approval still requires the code displayed in the app and the separate Operator token.

| Phase | Intended behavior and evidence | Status |
| --- | --- | --- |
| Implementation | Native Waiting state sends only bounded origin, request ID and expiry to the private local helper once per challenge. Helper asks the operator to type the displayed code, then uses existing Operator approval route. | Complete in source; exact-head CI pending |
| Static and contract | Validate socket ownership/path, exact loopback origin, bounded messages, no bearer/code in native handoff, dedup/rate/expiry and explicit confirmation. | Source reviewed; hosted checks pending |
| Feature functionality | CI tests real native Waiting transition and synthetic local helper protocol, invalid input/retry/cancel, and no effect in ordinary production launches. | Pending hosted checks |
| Native review | After matching Apple Silicon CI artifact, close the running app, rebuild/open with the local helper, and confirm a pairing request prompts the operator and proceeds only after confirmation. | Pending |
| Broader gates | Reuse unchanged backend/storage evidence; report any selected CI checks and GUI/Keychain evidence separately. | Pending |

The operator credential remains in a separate private file read by the helper only for a confirmed approval. A native app request alone never grants access. The helper is time bounded, local to the selected review, and has no production deployment role. No implementation build or test runs locally before native review; the task branch is pushed for CI.
