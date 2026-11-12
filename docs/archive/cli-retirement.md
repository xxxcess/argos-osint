# Retired terminal entry points

On 2026-11-12 the user authorized retirement of Venture, the six Fleetworks terminal clients (Atlas, Brain, Jobs, Tools, Models and Profile), and the legacy `argos` executable. The supported entry points are the [native macOS app](../../apps/fleetworks-desktop/README.md), [direct Docker backend](../../deploy/README.md) and separately configured [Intel Web](../wiki/code/intel-web.md). Backend service executables and administrative support scripts remain supported.

Historical TUI design, component, Profile, verification, screenshot and execution records are retained for provenance. Their commands and deleted source paths describe the implementation at the time; they are not current build, startup, import or recovery instructions. Existing user data, old deployment directories and caches are not deleted by executable retirement.

Retirement does not assert installed-app, accessibility, dual-Mac, both-Linux-architecture, full-workspace or real-provider verification. Previously recorded failures, skips and pending runtime checks remain evidence as originally recorded. Current checks are tracked separately in the named retirement plan.

See [current usage](../wiki/concepts/usage.md), [contributor workflow](../wiki/code/contributing.md) and the [archive index](index.md).

## Historical source links

Relative links into the removed `apps/venture`, `apps/fleetworks-cli` and `crates/argos-osint-bin` trees now point here. Their original labels and historical evidence are retained. The pre-retirement source remains in Git history (for example revision `8f34fa1c2c09bf687b6f4aab5d99356398e818ab`); use that revision when inspecting the earlier implementation. Removed TUI capture and legacy stack helpers likewise describe historical tooling, not current verification commands.
