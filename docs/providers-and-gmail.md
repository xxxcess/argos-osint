# Provider accounts and model roles

The Providers app separates **accounts** from **models**:

- **Grok**: subscription sign-in through Grok Build CLI only. Sign in with Grok
  opens browser sign-in; Check existing login reuses `grok login --oauth`.
  Install Grok Build CLI on `PATH`. Both Writer and Tools can use this account;
  model availability depends on account access. No API-key setup or fallback.
- **OpenAI**: ChatGPT subscription sign-in through Codex CLI only. No API-key
  setup. Sign in with ChatGPT displays the device verification URL/code; Check
  existing login reuses `codex login`. Install a recent Codex CLI on `PATH`.
- **OpenRouter**: OpenRouter API key, or `OPENROUTER_API_KEY`. API model IDs are
  provider-qualified, such as `vendor/model`. Attribution headers are preserved.
- **Models**: independent Writer (answers/reports) and Tools (research calls)
  cards. Choose each account and model here. Selections save immediately without
  modifying account credentials. ChatGPT is Writer-only; Tools supports Grok and
  OpenRouter. Existing local configurations remain selectable.
- **Sources**: existing OSINT source toggles and source-specific keys. Search
  service credentials never share an LLM key field.

OpenRouter API keys are masked. Enter edits a field; Ctrl+U clears its contents; paste is
supported. Save stores only the displayed account. Verify checks the form without
saving. OpenRouter verification checks its authenticated `/key` endpoint before
loading the public model catalog; an unsaved successful check says “Draft verified · Save to use”. Advanced
endpoint settings are optional and require HTTPS without embedded credentials.
Keys remain in owner-only `~/.argos/auth.json` (`ARGOS_HOME` overrides the root).
Legacy text/voice/Gmail data round-trips, and the existing text account is migrated
before another account can replace it. A corrupt auth file stops startup instead
of silently replacing existing credentials with empty defaults.

Enter on a model opens the selected account's catalog. Search or enter an exact
model ID; unavailable custom IDs are marked unverified. F5 refreshes the catalog.
Catalogs and asynchronous results stay scoped to their account. Selecting the
OpenRouter free router opens a list of concrete free models to pin. Ctrl+M,
`/model`, and `-m` target Writer. Both TUI and headless turns resolve role accounts
independently.

Grok sign-in runs `grok login --oauth` with API-key environment
variables removed and a five-minute timeout. Grok Build owns the OAuth credential
file; Argos reuses the existing token refresh adapter for model requests. API-only
Grok logins are rejected. Saved legacy Grok API keys remain untouched but are
ignored, including `XAI_API_KEY` and `GROK_API_KEY`. Checking a login also verifies
its model catalog; failed checks clear the old Grok catalog and show the error.
A spending-limit denial is shown as signed in with model access blocked, rather
than a failed login. Short terminals keep all subscription actions visible.
This does not guarantee model access for every Grok subscription tier.

ChatGPT access remains managed by Codex, including credential refresh, account
entitlements, and model availability. Argos checks for ChatGPT authentication
before running; an API-key Codex login is rejected for this mode. There is no API
billing fallback. Each answer uses `codex exec --ephemeral --json` in a temporary
workspace, ignoring user configuration and repository rules with read-only
permissions and shell, web, apps, plugins, and multi-agent features disabled.
Message chunks are forwarded into the existing answer/insight flow. A recent
Codex CLI is required; sign-in and completion have bounded timeouts. No live
provider calls are required by the test suite.

Mail and MCP configuration have been removed from Providers. The legacy Gmail
CLI/MCP backend remains compatible with previously configured installations;
this screen does not request mailbox credentials or generate MCP configuration.

## Voice

Ctrl+R records about five seconds with `rec` (from sox) or `ffmpeg`
(`avfoundation` input `:0` on macOS). The wav is posted to
`{voice base}/audio/transcriptions` as multipart form field `file`, model
`whisper-1` unless you set another. The transcript replaces the prompt. You
send it with Enter. If neither recorder is installed, the stream says so and
nothing is invented.
