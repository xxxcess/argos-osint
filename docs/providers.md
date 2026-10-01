# Providers

Providers has Grok, OpenAI, OpenRouter, and Defaults tabs. Connections supply credentials; Defaults independently selects a provider and model for three roles: Recon (questions and bindings), Tool picker (tool order), and Synthesis (answers). Changing or checking a connection does not change any role. Existing Writer configuration initializes Recon and Synthesis during migration. The Tool picker defaults to OpenRouter with `typesafe/jev-1.13`, seeded only when its provider and model are both empty.

Grok uses Grok Build subscription sign-in through `grok login --oauth`. OpenAI uses ChatGPT subscription sign-in through Codex CLI device authorization. OpenRouter uses an API key or `OPENROUTER_API_KEY`; its optional HTTPS endpoint is configurable. Verification checks the current draft without saving it. Account credentials are stored in `~/.argos/auth.json` with owner only permissions on Unix. Provider authentication and model usage follow the provider's own terms.

```sh
argos login
argos defaults show
argos defaults set recon --provider openrouter --model <model-id>
argos defaults set tool-picker --provider openrouter --model typesafe/jev-1.13
argos defaults set synthesis --provider openrouter --model <model-id>
argos models --role tool-picker
argos models --role synthesis
argos logout
```

The Recon role derives the turn's three questions and extracts input bindings from observations. The Tool picker role picks one tool per request until the ordered list is complete. Jev decisions models (`typesafe/jev-1.13`, `~typesafe/jev-latest`, or an id containing `/jev`) use `POST https://openrouter.ai/api/alpha/decisions` with the OpenRouter key; any other model uses the chat transport with a JSON reply. `defaults show` reports `tool_picker: { provider, model, transport }` with `transport` set to `decisions` or `chat`. `tool-picker` and `tool_picker` both name the role. Pin the concrete Jev id rather than the `~typesafe/jev-latest` alias so a later release cannot change picker behavior without a Defaults change. The Synthesis role writes cited answers and extracts evidence-backed claims. On Defaults, a three-way selector (Recon, Tool picker, Synthesis) chooses the role, and Provider and Model open a list for that role only. Saving a role writes `config.toml` only, never credentials, and the System event log records the change (for example `defaults.tool_picker: … -> …`). For the Tool picker on OpenRouter, the model list always offers `Jev 1.13 (decisions)` first, even when `GET /api/v1/models` omits it; `argos models --role tool-picker` does the same. The provider list is the accounts connected on this machine (Grok subscription, ChatGPT sign-in, an OpenRouter key, and Local). The model list is what that account is allowed to call. ChatGPT subscription selects the Codex default. A saved model identifier is retained if a live catalog is unavailable. `argos osint user-agent` sets the identifying contact string for public HTTP services separately from model defaults.
