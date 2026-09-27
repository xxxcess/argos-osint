# Case Desk, evidence, and research

Ordinary Case Desk questions retrieve completed report passages before invoking
the writer. The default Desk scope includes reports without a case; an open
report defaults to that report. Expand deliberately with `/scope case <id>`,
`/scope reports <ids>`, or `/scope collection`. Results show relevance separately
from report dates. Exact domains, IPs, and email addresses require exact passage
matches. Aliases remain inside the selected scope.

An answer cites `report-id@vN:Lline`. `/cite 1` opens the first recommendation;
`/cite <citation>` also resolves citations retained from earlier versions. The
reader keeps the originating question and its line position. `g` opens the
network and focuses a matching mention when its source span still matches the
latest report. Historic text is separate from the latest graph source. Esc
returns to the Desk prompt and scroll position. Report discussion is temporary;
`/retain-answer` explicitly saves an answer as a report-tagged fact.

No matching passage produces an evidence gap, not an invented answer. A model
is optional: without one, attributed passages and their citations remain
available. Evidence-only turns advertise no tool definitions and reject tool
calls returned by a model. Source text is never authority to execute a tool.

## Research configuration

Open Providers → Research. Select an integration, enable it, configure its
limits/scope, and save. Model-provider accounts remain independent. Credentials
are masked and stored by reference in `auth.json`, not in ordinary research
configuration. Test Configuration is offline and reports what it could verify;
it does not spend API credits or prove account entitlement.

| Integration | Current execution path | Setup / limits |
| --- | --- | --- |
| Search | Existing Facts/Web/News/Social and configured search sources | Configure Sources and keys as needed. Search operators depend on the selected provider. |
| Domain | DNS, RDAP, certificates, Wayback and InternetDB | Select an exact public domain. Shared hosting is not ownership evidence. |
| InternetDB | Keyless HTTP with existing parser | Exact public IP; returns previously observed services, not verified current exposure. |
| Identity | Selected GitHub profile references | Optional GitHub token; profile/name matches remain candidates. |
| Published contacts | Scoped native page fetch and email spans | Enable active HTTP and allow the exact host; currently the root page only. |
| LeakCheck Public | Keyless exposure lookup with attribution | Explicit sensitive-lookup opt-in and shared provider rate gate. No credentials are retrieved. |
| Shodan | Authenticated minified host lookup and structured parser | Key reference and host-lookup account entitlement required. Quota display remains unverified. |
| XposedOrNot | Basic email breach analytics and dated/category parser | Sensitive-lookup opt-in; conservative provider-wide rate gate. Domain monitoring is unavailable. |
| WhatsMyName | Native selected-site response checking | Supply a versioned dataset JSON path and selected site names. Matches are provisional. Dataset refresh is manual. |
| Katana | Managed installation/version/help verification | Collection is unavailable until external-tool network isolation and installed output are verified. |
| SpiderFoot / Mosint / Maigret | Configuration and executable version checks | Collection and automatic installation remain unavailable. Use an isolated executable/container manually; no system Python is modified. |

Shodan and XposedOrNot parsing use their official documented contracts and
fixtures, without routine live lookups. They have not been live-verified with
this installation's account or network. The local tools have not been installed
or claimed operational.

`/investigate <integration> [input]` submits explicit enrichment to the shared
research queue. With no input it uses the selected entity. `i` opens cached
evidence and actions; cursor movement does not start network work. `/jobs`
shows progress/errors; `/cancel-jobs` cancels queued and running jobs.

Research inputs select eligible logical stages. Unsuitable stages are skipped,
independent jobs run within global/provider limits, and partial results survive
adapter failures. Automatic broad recursive discovery is not enabled; search
discoveries need analyst selection before further pivots. External aggregators
fail closed because indirect provider requests and target scope cannot yet be
enforced. Native HTTP validates redirects, resolves and pins public addresses,
bounds output, and shares request caching/rate gates across adapter calls.

## Managed tool actions

Install and Update show a reviewable version/source/destination/prerequisite
plan. Apply starts a visible background job. The supported managed installer
is Katana 1.7.0 on macOS/Linux amd64/arm64, using official release assets and
their SHA-256 checksum file. It extracts only the named binary and verifies
version and JSONL/scope/depth flags before activating a new directory. Failure
retains the configured previous installation. Verify checks without collection.
Remove accepts only an Argos-managed directory with its manifest. Browser,
container runtime, and system Python installation are never automatic.

Offline operation, unsupported platforms, or inaccessible releases produce
actionable failures. No tool download/install is triggered by opening Providers.

## Review, report versions, and graphs

`/findings` labels new, repeated, changed, and candidate identity observations.
Changed observations are not automatically labeled contradictions. Explicitly
flagged conflicts retain their attribution. Typed relationships and their
uncertainty are visible separately. `/review <id> retain|accept|reject|defer
<reason>` keeps a decision history and requires the observation to be in scope.

`/save-update addendum|revision|followup` writes accepted findings with source
attribution. A new file/version is activated without rewriting the original;
earlier citations remain resolvable. Already incorporated observations are not
replayed on another save. Indexes and affected graph snapshots refresh.

`/correct <source-byte-offset> <replacement_or_suppress> <reason>` persists an
extraction correction; underscores in a replacement represent spaces. The
recorded original text must still match. `/undo-correction <id>` reverses it.
`/merge <canonical-node-id> <other-node-ids>` records an explicit identity
decision for current report entities; `/unmerge <decision-id>` reverses it.
Original labels, source mentions, and history are retained through rebuilds.

Cockpit remains the default. Clusters show research coverage separately from
missing co-occurrence. Path exposes hop provenance and bounded searches;
`/corroborate` and `/weakest` focus corroboration work. Matrix retains adjacency
and adds theme-by-report coverage with `c`; a cell opens its supporting passage.
Ribbon retains accepted/rejected source extraction. The same cached inspector
serves the views. Graph caps and display limits are visible. Co-occurrence is
never promoted implicitly to ownership, verified identity, or confidence.

`/timeline` aligns provider observations and report statements as textual
lanes. Event dates remain distinct from report publication and retrieval.
Report statements without structured event dates remain undated, and snapshot
build time is never event time. A graphical timeline and dedicated earlier/later
comparison surface remain follow-up work.

## Migration and validation

SQLite migrations are additive: versioned report text, passages/FTS5, structured
evidence, review decisions, and persistent research jobs. Existing reports are
indexed from their saved files without new research. Missing files are reported
as unavailable. Index rules and TNA pipeline versions invalidate derived data.
Legacy research credentials move into owner-only `auth.json`; conflicts preserve
both current and legacy secrets. Desk transcripts are retained across restart.
Interrupted jobs become partial instead of being presented as still running.

Automated tests use fixtures and local mocks. They cover old citation retention,
exact observable retrieval, scope boundaries, correction/merge reversals,
report-update deduplication, secret migration/redaction, process output bounds
and timeout, restart recovery, rate gates, and normal/narrow TUI layouts. No
broad scan, paid API lookup, or automatic tool installation is a routine test.

Remaining work includes advanced provider-specific search templates, automatic
validated downstream pivots, Enrich on Review, complete multi-report occurrence
comparison, richer semantic conflict detection, graphical timeline comparison,
account quota discovery, dataset refresh, and verified isolated CLI collection.

## Official contract references

- [Katana 1.7.0 release](https://github.com/projectdiscovery/katana/releases/tag/v1.7.0) and [CLI documentation](https://docs.projectdiscovery.io/opensource/katana/running).
- [Shodan API](https://developer.shodan.io/api) and [host lookup entitlement](https://help.shodan.io/developer-fundamentals/looking-up-ip-info).
- [XposedOrNot endpoints, limits, and response examples](https://xposedornot.com/api_doc).
- [WhatsMyName dataset](https://github.com/WebBreacher/WhatsMyName).
- [Mosint](https://github.com/alpkeskin/mosint), [Maigret CLI](https://maigret.readthedocs.io/en/latest/command-line-options.html), and [SpiderFoot 4.0 CLI](https://github.com/smicallef/spiderfoot/blob/v4.0/sf.py).
- [IANA TLD list](https://data.iana.org/TLD/tlds-alpha-by-domain.txt), bundled for deterministic domain validation.
