# Request-mode / Defaults popup checklist (PR #47)

> Historical record. For current behavior, start at the [wiki](../index.md).
- [x] Root cause of screenshot 404s found live (Ultra undeployed for the account; not a request-mode issue)
- [x] Deployment-missing 404 classified as InvalidModel with masked detail
- [x] Stage-wide hard-reject memory by model family + 1800 s breaker
- [x] Last live route overload retries; cooling-vs-dead bug fixed
- [x] Mode ladder on gateway "No endpoints found" 404
- [x] Defaults popups for all 9 roles + TestBackend test
- [x] Live Atlas + 8 articles through the brief; per-route errors recorded
- [x] Matrix (default, no-default-features, nlp) with clippy -D warnings and fmt
- [x] Atlas: transient retry, same-host redirects, quota counted separately, failures named in status
- [x] Ultra history analysed (not request-shape dependent: 404 in the afternoon, 503/500 overloaded in the evening; recon uses Super)
- [x] Grok (xAI) removed with migration + tests
- [ ] Persist learned request-mode capabilities (open)
- [ ] User action: replace Ultra with Super / check Gemini quota
