# Intel Web connection boundary

Intel Web is an independent browser application. The native app registers its
workstation/managed child with a configured gateway, then opens this deployment's
`/connect` form. The form accepts an existing Connect ID; it never creates a node,
workspace or registration. Connect IDs select context and do not authenticate a
person or grant administration rights.

The custom server, protected server rendering, REST adapter and live upgrades use
the same authoritative gateway session resolver. Missing or revoked sessions deny
data before rendering/forwarding. An opaque session lives only in a host-only,
HttpOnly, SameSite=Strict cookie. Remote HTTPS adds Secure; loopback HTTP is the
documented local development exception. No browser bearer, localStorage unlock or
developer bypass exists.

The server's API credential needs WebRelay purpose and ResolveWebSession
permission. It is separate from native registration/maintenance credentials.
Configure `FLEETWORKS_API_ORIGIN` and a protected
`FLEETWORKS_API_TOKEN_FILE` through external deployment tooling. The ordinary
browser API allowlist does not forward native registration, dataset uploads,
maintenance or private index operations.

`FLEETWORKS_WEB_ORIGIN` is the exact public origin; it is independent of private
upstream addresses. A remote origin must be HTTPS behind authenticated ingress.
Set `FLEETWORKS_WEB_AUTHENTICATED_INGRESS=1` and provision a private, owner-only
`FLEETWORKS_WEB_INGRESS_TOKEN_FILE`. The existing authenticated proxy must strip
any incoming `x-fleetworks-ingress-token` and inject this protected forwarding
token only after its normal authentication. The web server validates it on HTTP
and WebSocket boundaries. Arbitrary Forwarded/X-Forwarded headers grant no trust.
This forwarding secret is neither a browser credential nor the gateway bearer.
Actual ingress authentication, TLS and remote reachability still require operator
integration verification.

Optional `FLEETWORKS_WEB_BASE_PATH` is a validated path such as `/intel`. Set it
consistently when building Next and running the server; it affects compiled asset
paths, cookie scope, connection routes, browser API/live URLs and navigation.
Changing it requires the matching web build, not only a runtime URL change.

New requests retain the mounted context's expected revision. Switching context in
another tab cannot silently retarget a brief submission: stale requests receive
409 and reconnect. Successful switching revokes the previous cookie's session;
tabs discard their Query clients/live subscriptions after change notification,
expiry, or an authoritative denial. The server revalidates live authority every
five seconds, stops forwarding after ten seconds without verified authority, and
stops at expiry. App/browser closure does not cancel accepted durable jobs.

Local tests must provision their own authority. The external test harness
`__tests__/serve-connect-fixture.mjs --port <port> [--dev]` starts a synthetic
gateway and the real server through normal credentials/cookies. It does not add
a production flag/header or contact user services. Guard/REST/live/browser
fixtures are separate evidence from real SurrealDB registry persistence, Linux
jobs, public ingress and topology visual review. Repository Playwright output
remains isolated with `ARGOS_WEB_TEST_DEV=1`.

Implementation and remaining cutover acceptance are tracked in
`.planning/2026-11-11-rune-macos-cutover/handoff.md` and the canonical wiki routes.
