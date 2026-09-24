# Providers, voice, and Gmail

Argos does not bind the agent loop to one model vendor. Every signed-in
provider is called the same way: `POST {base}/chat/completions` for text and
`POST {base}/audio/transcriptions` for voice. Login chooses Grok, OpenAI,
OpenRouter, or a local server, and that choice only changes the host, the
default model, the key, and OpenRouter's attribution headers.

## Terminal login

`argos login` prints the four providers, then asks:

1. Modality, `text` or `voice`. Each slot is independent.
2. Provider, `grok`, `openai`, `openrouter`, or `local`.
3. Base URL. The default is that provider's public API. Change it only for a proxy or a local port.
4. Model. Defaults are `grok-4.6`, `gpt-4.1`, `openai/gpt-4.1`, and `llama3.2`. Voice defaults are `whisper-1`, `openai/whisper-1`, or `whisper`.

Nothing saved means the text slot is already Grok on `https://api.x.ai/v1`
with `grok-4.6`. That is the model id in Grok Build's `default_models.json`.
`Ctrl+M`, `/model`, and `argos models` list `grok-4.6` and `grok-4.5`, then
merge the live `/v1/models` catalog when the key can reach the endpoint.
`/model grok-4.5` and `argos -m grok-4.5` select one. The choice is stored in
`~/.argos/config.toml` as `model`. Chat still uses chat completions, which
is the API Grok documents for `grok-4.6` alongside the Responses API.
5. API key, read with echo off.

Cloud providers require a key. When `XAI_API_KEY`, `OPENAI_API_KEY`, or
`OPENROUTER_API_KEY` is already set, the prompt offers to use that variable
and does not copy it into `auth.json`. A local server may leave the key empty.

The Providers app edits the same file. Enter on the provider row cycles the
four ids and refreshes the URL and model while they still match a preset.
The key field is masked. "Test /models" calls `GET {base}/models` with the
same key and headers a chat turn would use.

OpenRouter requests include `HTTP-Referer`, `X-Title`, and
`X-OpenRouter-Title`. Grok and OpenAI use bearer auth only. A saved kind of
`api` from an older file is classified from the host, so an OpenRouter URL
still gets those headers.

## Voice

Ctrl+R records about five seconds with `rec` (from sox) or `ffmpeg`
(`avfoundation` input `:0` on macOS). The wav is posted to
`{voice base}/audio/transcriptions` as multipart form field `file`, model
`whisper-1` unless you set another. The transcript replaces the prompt. You
send it with Enter. If neither recorder is installed, the stream says so and
nothing is invented.

## Gmail

Google app passwords are the credential Odysseus documents for IMAP
username/password accounts. Outlook-style OAuth is out of scope. Argos pins
the host to `imap.gmail.com` port 993 and strips spaces out of the app
password before login.

The Gmail app actions:

* Save, after the address and password pass a local check.
* Test INBOX, which selects the mailbox and reports how many messages exist.
* Write MCP config, which stores `~/.argos/mcp.json` pointing at
  `argos mcp gmail`.

The stdio server accepts one JSON-RPC message per line, and also a
`Content-Length` frame. `initialize`, `tools/list`, `tools/call`, and `ping`
are implemented. Tools:

* `gmail_list_recent` with optional `limit` (1–20)
* `gmail_search` with `query` and optional `limit`

Search text cannot contain quotes or line breaks. The tools return headers
(uid, date, from, subject), not a mailbox export, and they never send mail.
Logs go to stderr. stdout is reserved for MCP messages.

The agent may list recent headers when you ask about Gmail and an account is
saved. The app password stays in `auth.json` and is not copied into the prompt.
