# Providers

Providers has four clickable tabs: Grok, OpenAI, OpenRouter, and Models.

Grok uses Grok Build subscription sign-in. **Sign in with Grok** runs `grok login --oauth` and shows its browser instructions. **Check existing login** reuses an existing Grok session and checks model access. API keys do not stand in for Grok subscription access.

OpenAI uses ChatGPT subscription sign-in through Codex CLI. **Sign in with ChatGPT** starts device authorization and displays its URL and code. **Check existing login** checks the Codex subscription session. OpenAI does not ask for an API key here.

OpenRouter has a masked API key field. **Verify connection** checks the form against OpenRouter without saving it, and **Save key** stores only the OpenRouter account. **Show advanced endpoint** reveals an editable HTTPS API endpoint. An empty key uses `OPENROUTER_API_KEY` from the environment.

Models assigns the Writer provider and model separately from account setup. Account credentials are stored in `~/.argos/auth.json` with owner only permissions on Unix. `argos login`, `argos models`, and `argos logout` provide command line access.
