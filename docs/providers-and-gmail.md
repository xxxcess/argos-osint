# Providers, voice, and Gmail

## Terminal login

`argos login` asks, in order:

1. Modality, `text` or `voice`.
2. Kind, `local`, `api`, or `device`.
3. Base URL and model.
4. A secret that matches the kind.

An API key is read with echo off. A local endpoint may omit the key. A
device login asks for the OAuth client id, the device-authorization URL, and
the token URL, then prints the user code and verification URL from RFC 8628
and polls the token endpoint until you approve, deny, or the code expires.
Slow-down responses wait longer. The access token is stored as the slot's
API key and is not printed back.

The Providers app in the TUI edits the same `~/.argos/auth.json` file. The
key field is masked. "Test /models" calls `GET {base}/models`. "Start
device-code login" runs the same poll and writes the token when it arrives.

Text completions are `POST {base}/chat/completions`, streamed when the server
allows it, with a non-streaming retry. Tool calls use the OpenAI function
shape.

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
