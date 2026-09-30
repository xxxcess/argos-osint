# Providers

Providers has Grok, OpenAI, OpenRouter, and Defaults tabs. Connections supply credentials; Defaults independently selects a provider and model for Recon planning and Synthesis. Changing or checking a connection does not change either role. Existing Writer configuration initializes Synthesis during migration.

Grok uses Grok Build subscription sign-in through `grok login --oauth`. OpenAI uses ChatGPT subscription sign-in through Codex CLI device authorization. OpenRouter uses an API key or `OPENROUTER_API_KEY`; its optional HTTPS endpoint is configurable. Verification checks the current draft without saving it. Account credentials are stored in `~/.argos/auth.json` with owner only permissions on Unix. Provider authentication and model usage follow the provider's own terms.

```sh
argos login
argos defaults show
argos defaults set recon --provider openrouter --model <model-id>
argos defaults set synthesis --provider openrouter --model <model-id>
argos models --role synthesis
argos logout
```

The Recon role submits a native plan function where supported and falls back to structured JSON, including subscription completion paths that cannot accept native tool specs. The Synthesis role writes cited answers and extracts evidence-backed claims. Model lists are refreshed from the selected provider when available; a saved model identifier is retained if a live catalog is unavailable. `argos osint user-agent` sets the identifying contact string for public HTTP services separately from model defaults.
