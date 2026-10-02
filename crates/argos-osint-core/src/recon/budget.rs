//! Dynamic turn deadline. The allowance is recomputed as rounds add calls, and synthesis
//! keeps its own time even when tools run long.
use std::time::{Duration, Instant};

use crate::osint::{self, COURTLISTENER_SPACING, LEGAL_TOOLS};

/// Seconds added for each Recon or tool-picker model round that actually runs.
pub const RECON_ROUND_SECONDS: u64 = 45;
/// Synthesis starts with this many seconds, then one more per 1,000 characters.
pub const SYNTHESIS_BASE_SECONDS: u64 = 300;
/// Room left for Recon and tools when the turn ceiling is too small to hold the
/// full synthesis minimum and still run a call.
const PHASE_WINDOW_SECONDS: u64 = 120;
/// Streaming synthesis ends when this long passes with no new text.
pub const SYNTHESIS_IDLE_SECONDS: u64 = 60;
/// Executor concurrency (`Semaphore::new(4)`). Tool wall time divides by this.
pub const TOOL_CONCURRENCY: u64 = 4;

pub const RECON_DEADLINE: &str = "recon deadline reached";
pub const TURN_BUDGET: &str = "turn budget";
pub const SYNTHESIS_DEADLINE: &str = "synthesis deadline reached";
pub const SYNTHESIS_IDLE: &str = "synthesis idle timeout";
pub const CUT_SHORT: &str = "Synthesis was cut short";
pub const CUT_NOTE: &str = "Synthesis ran out of time; re-run or raise max_turn_seconds.";
/// The provider closed the stream after some answer text had already arrived.
pub const STREAM_LOST: &str = "provider stream ended";

/// One scheduled call. A cache hit contributes nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScheduledCall {
    pub tool_id: String,
    pub timeout_seconds: u64,
    pub cached: bool,
    /// Firecrawl job polling, on top of the request timeout. Zero when the tool does not poll.
    pub poll_seconds: u64,
}

/// A live call as the catalog defines it. Cache hits are marked by the caller.
pub fn scheduled(tool_id: &str, cached: bool) -> ScheduledCall {
    let id = osint::canonical_tool_id(tool_id);
    let timeout = osint::definition(id).map(|tool| tool.timeout_seconds).unwrap_or(30);
    ScheduledCall {
        tool_id: id.to_string(),
        timeout_seconds: timeout,
        cached,
        poll_seconds: if cached { 0 } else { osint::job_poll_seconds(id) },
    }
}

/// `max(longest timeout, sum(timeouts) / 4)` plus CourtListener spacing and Firecrawl polling.
/// Cache hits add nothing.
pub fn tool_allowance_seconds(calls: &[ScheduledCall]) -> u64 {
    let live: Vec<&ScheduledCall> = calls.iter().filter(|call| !call.cached).collect();
    if live.is_empty() {
        return 0;
    }
    let longest = live.iter().map(|call| call.timeout_seconds).max().unwrap_or(0);
    let sum: u64 = live.iter().map(|call| call.timeout_seconds).sum();
    let concurrent = longest.max(sum / TOOL_CONCURRENCY);
    let court = live
        .iter()
        .filter(|call| LEGAL_TOOLS.contains(&call.tool_id.as_str()))
        .count();
    let spacing = court.saturating_sub(1) as u64 * COURTLISTENER_SPACING.as_secs();
    let polling: u64 = live.iter().map(|call| call.poll_seconds).sum();
    concurrent + spacing + polling
}

/// 300s plus 1s per 1,000 characters, capped by `ceiling`. A repair pass adds half of
/// that again, still not past the ceiling.
pub fn synthesis_allowance_seconds(chars: usize, repair: bool) -> u64 {
    synthesis_allowance_capped(chars, repair, u64::from(crate::provider::MAX_MAX_TURN_SECONDS))
}

pub fn synthesis_allowance_capped(chars: usize, repair: bool, ceiling: u64) -> u64 {
    let ceiling = ceiling.max(1);
    let grown = SYNTHESIS_BASE_SECONDS.saturating_add((chars as u64) / 1_000);
    let capped = grown.min(ceiling);
    if repair {
        capped.saturating_add(capped / 2).min(ceiling)
    } else {
        capped
    }
}

/// `clamp(recon + tools + synthesis, floor, ceiling)`. The ceiling is at least the floor.
pub fn deadline_seconds(recon: u64, tools: u64, synthesis: u64, floor: u64, ceiling: u64) -> u64 {
    let ceiling = ceiling.max(floor);
    recon.saturating_add(tools).saturating_add(synthesis).clamp(floor, ceiling)
}

pub fn format_span(total: u64) -> String {
    let minutes = total / 60;
    let seconds = total % 60;
    if minutes == 0 {
        format!("{seconds}s")
    } else if seconds == 0 {
        format!("{minutes}m")
    } else {
        format!("{minutes}m {seconds}s")
    }
}

/// `Deadline 6m 10s: 11 calls, ~52k chars evidence`. The evidence clause is omitted until
/// synthesis has measured the packet.
pub fn format_deadline(total: u64, calls: usize, evidence_chars: usize) -> String {
    let time = format_span(total);
    if evidence_chars == 0 {
        format!("Deadline {time}: {calls} calls")
    } else {
        let thousands = evidence_chars.div_ceil(1000);
        format!("Deadline {time}: {calls} calls, ~{thousands}k chars evidence")
    }
}

/// Wall-clock budget for one turn. Phase checks read this; callers emit [`Self::take_labels`]
/// after each change.
pub struct TurnClock {
    pub started: Instant,
    pub floor: Duration,
    pub ceiling: Duration,
    pub recon_rounds: u32,
    calls: Vec<ScheduledCall>,
    /// Tool allowance, raised when the plan grows and never lowered when steps are skipped.
    tool_hold: u64,
    pub evidence_chars: usize,
    pub repair: bool,
    pub tools_started: Option<Instant>,
    pub synthesis_started: Option<Instant>,
    pub idle: Duration,
    /// Test hook: replace the computed synthesis allowance.
    pub synthesis_override: Option<u64>,
    last_label: String,
    pending: Vec<String>,
}

impl TurnClock {
    pub fn new(floor_secs: u64, ceiling_secs: u64) -> Self {
        let ceiling = ceiling_secs.max(floor_secs);
        Self {
            started: Instant::now(),
            floor: Duration::from_secs(floor_secs),
            ceiling: Duration::from_secs(ceiling),
            recon_rounds: 0,
            calls: Vec::new(),
            tool_hold: 0,
            evidence_chars: 0,
            repair: false,
            tools_started: None,
            synthesis_started: None,
            idle: Duration::from_secs(SYNTHESIS_IDLE_SECONDS),
            synthesis_override: None,
            last_label: String::new(),
            pending: Vec::new(),
        }
    }

    pub fn note_round(&mut self) {
        self.recon_rounds = self.recon_rounds.saturating_add(1);
        self.queue();
    }

    pub fn raise_calls(&mut self, calls: Vec<ScheduledCall>) {
        let allowance = tool_allowance_seconds(&calls);
        if allowance >= self.tool_hold {
            self.tool_hold = allowance;
        }
        self.calls = calls;
        self.queue();
    }

    pub fn set_evidence(&mut self, chars: usize) {
        self.evidence_chars = chars;
        self.queue();
    }

    pub fn mark_repair(&mut self) {
        self.repair = true;
        self.queue();
    }

    pub fn begin_tools(&mut self) {
        if self.tools_started.is_none() {
            self.tools_started = Some(Instant::now());
        }
    }

    pub fn begin_synthesis(&mut self) {
        if self.synthesis_started.is_none() {
            self.synthesis_started = Some(Instant::now());
        }
        self.queue();
    }

    /// Moves the turn start backward so tests can age a phase without sleeping.
    #[cfg(test)]
    pub fn age(&mut self, by: Duration) {
        self.started = self.started.checked_sub(by).unwrap_or(self.started);
        if let Some(at) = self.tools_started {
            self.tools_started = Some(at.checked_sub(by).unwrap_or(at));
        }
        if let Some(at) = self.synthesis_started {
            self.synthesis_started = Some(at.checked_sub(by).unwrap_or(at));
        }
    }

    pub fn recon_seconds(&self) -> u64 {
        u64::from(self.recon_rounds) * RECON_ROUND_SECONDS
    }

    pub fn tool_seconds(&self) -> u64 {
        self.tool_hold
    }

    pub fn synthesis_seconds(&self) -> u64 {
        if let Some(seconds) = self.synthesis_override {
            return seconds;
        }
        let ceiling = self.ceiling.as_secs();
        if self.repair {
            return synthesis_allowance_capped(self.evidence_chars, true, ceiling);
        }
        if self.synthesis_started.is_none() && self.evidence_chars == 0 {
            return SYNTHESIS_BASE_SECONDS.min(ceiling);
        }
        synthesis_allowance_capped(self.evidence_chars, false, ceiling)
    }

    /// Synthesis time held back from Recon and tools. A tight ceiling keeps a window
    /// so those phases can still run.
    fn synthesis_hold(&self) -> u64 {
        let wanted = self.synthesis_seconds();
        let ceiling = self.ceiling.as_secs();
        if ceiling.saturating_sub(wanted) < PHASE_WINDOW_SECONDS {
            ceiling.saturating_sub(PHASE_WINDOW_SECONDS)
        } else {
            wanted
        }
    }

    pub fn deadline(&self) -> Duration {
        Duration::from_secs(deadline_seconds(
            self.recon_seconds(),
            self.tool_seconds(),
            self.synthesis_seconds(),
            self.floor.as_secs(),
            self.ceiling.as_secs(),
        ))
    }

    pub fn label(&self) -> String {
        let calls = self.calls.iter().filter(|call| !call.cached).count();
        let chars = if self.synthesis_started.is_some() {
            self.evidence_chars
        } else {
            0
        };
        format_deadline(self.deadline().as_secs(), calls, chars)
    }

    /// Saved on the plan. Starts with the same text the TUI shows.
    pub fn breakdown(&self) -> String {
        format!(
            "{} (recon {}s, tools {}s, synthesis {}s)",
            self.label(),
            self.recon_seconds(),
            self.tool_seconds(),
            self.synthesis_seconds()
        )
    }

    pub fn take_labels(&mut self) -> Vec<String> {
        std::mem::take(&mut self.pending)
    }

    /// Time left for a Recon or picker round before synthesis's reserve and the deadline.
    pub fn recon_remaining(&self) -> Duration {
        self.deadline()
            .saturating_sub(self.started.elapsed())
            .saturating_sub(Duration::from_secs(self.synthesis_hold()))
    }

    /// Synthesis allowance still unused, and never past the hard ceiling.
    pub fn synthesis_remaining(&self) -> Duration {
        let allowance = Duration::from_secs(self.synthesis_seconds());
        let used = self.synthesis_started.map(|at| at.elapsed()).unwrap_or_default();
        allowance.saturating_sub(used).min(self.ceiling_remaining())
    }

    pub fn ceiling_remaining(&self) -> Duration {
        self.ceiling.saturating_sub(self.started.elapsed())
    }

    /// The tool phase has used its allowance (plus any floor slack), or the ceiling would
    /// eat the synthesis reserve. In-flight calls are left to finish; new ones are not launched.
    pub fn tools_blocked(&self) -> bool {
        let Some(started) = self.tools_started else {
            return false;
        };
        let allowance = Duration::from_secs(self.tool_seconds());
        let reserve = self.synthesis_hold();
        let sum = self
            .recon_seconds()
            .saturating_add(self.tool_seconds())
            .saturating_add(reserve);
        let slack = self.deadline().as_secs().saturating_sub(sum);
        if started.elapsed() >= allowance + Duration::from_secs(slack) {
            return true;
        }
        self.started.elapsed() + Duration::from_secs(reserve) >= self.ceiling
    }

    fn queue(&mut self) {
        let label = self.label();
        if self.last_label == label {
            return;
        }
        self.last_label = label.clone();
        self.pending.push(label);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn live(id: &str) -> ScheduledCall {
        scheduled(id, false)
    }

    #[test]
    fn more_calls_mean_more_tool_time_and_cache_hits_add_nothing() {
        let one = live("crtsh_certificates");
        // Four calls still fit in one concurrency slot; the fifth makes the sum exceed the longest timeout.
        let five = tool_allowance_seconds(&[one.clone(), one.clone(), one.clone(), one.clone(), one.clone()]);
        assert!(five > tool_allowance_seconds(std::slice::from_ref(&one)));
        let cached = scheduled("crtsh_certificates", true);
        assert_eq!(tool_allowance_seconds(std::slice::from_ref(&cached)), 0);
        assert_eq!(tool_allowance_seconds(&[cached, one.clone()]), tool_allowance_seconds(std::slice::from_ref(&one)));
    }

    #[test]
    fn courtlistener_spacing_and_firecrawl_polling_are_counted() {
        let case = live("courtlistener_case_search");
        let one = tool_allowance_seconds(std::slice::from_ref(&case));
        let two = tool_allowance_seconds(&[case.clone(), case.clone()]);
        assert_eq!(two, one + COURTLISTENER_SPACING.as_secs());
        assert_eq!(one, case.timeout_seconds, "one call has no spacing gap");
        let job = live("firecrawl_batch_scrape");
        let without = ScheduledCall { poll_seconds: 0, ..job.clone() };
        assert!(job.poll_seconds > 0);
        assert_eq!(tool_allowance_seconds(std::slice::from_ref(&job)), tool_allowance_seconds(std::slice::from_ref(&without)) + job.poll_seconds);
        let crawl = live("firecrawl_crawl");
        assert!(crawl.poll_seconds >= 120);
        assert_eq!(tool_allowance_seconds(&[scheduled("firecrawl_search", false)]).saturating_sub(60), 0);
    }

    #[test]
    fn bigger_evidence_means_more_synthesis_time_until_the_ceiling() {
        assert_eq!(synthesis_allowance_capped(0, false, 900), 300);
        assert_eq!(synthesis_allowance_capped(2_500, false, 900), 302);
        assert_eq!(synthesis_allowance_capped(1_000_000, false, 900), 900);
        assert_eq!(synthesis_allowance_capped(0, true, 900), 450);
        assert_eq!(synthesis_allowance_seconds(0, false), 300);
        assert_eq!(synthesis_allowance_seconds(1_000_000, false), 1_300);
    }

    #[test]
    fn a_round_keeps_its_time_and_a_tight_ceiling_still_runs_tools() {
        let mut clock = TurnClock::new(300, 900);
        assert_eq!(clock.synthesis_seconds(), 300);
        clock.note_round();
        let remaining = clock.recon_remaining().as_secs();
        assert!((44..=45).contains(&remaining), "a round keeps about 45s, got {remaining}");
        clock.raise_calls(vec![live("crtsh_certificates")]);
        clock.begin_tools();
        assert!(!clock.tools_blocked());
        let mut tight = TurnClock::new(300, 300);
        tight.raise_calls(vec![live("crtsh_certificates")]);
        tight.begin_tools();
        assert!(!tight.tools_blocked(), "a 300s ceiling still leaves a tool window");
    }

    #[test]
    fn floor_and_ceiling_hold() {
        assert_eq!(deadline_seconds(10, 10, 10, 300, 900), 300);
        assert_eq!(deadline_seconds(200, 200, 200, 300, 900), 600);
        assert_eq!(deadline_seconds(400, 400, 400, 300, 900), 900);
        assert_eq!(deadline_seconds(10, 10, 10, 800, 120), 800, "ceiling is raised to the floor");
    }

    #[test]
    fn the_transcript_label_matches_the_example() {
        assert_eq!(
            format_deadline(370, 11, 52_000),
            "Deadline 6m 10s: 11 calls, ~52k chars evidence"
        );
        assert_eq!(format_deadline(300, 0, 0), "Deadline 5m: 0 calls");
        assert_eq!(format_span(45), "45s");
    }

    #[test]
    fn aging_the_tool_clock_blocks_new_calls_and_cache_hits_do_not_extend_it() {
        let mut clock = TurnClock::new(30, 900);
        clock.raise_calls(vec![live("courtlistener_case_search"), live("courtlistener_case_search")]);
        clock.begin_tools();
        assert!(!clock.tools_blocked());
        let held = clock.tool_seconds();
        clock.raise_calls(vec![scheduled("courtlistener_case_search", true)]);
        assert_eq!(clock.tool_seconds(), held, "a later cache-only refresh does not shrink the hold");
        clock.age(Duration::from_secs(held + 1));
        assert!(clock.tools_blocked());
    }

    #[test]
    fn the_label_updates_when_a_round_adds_calls_and_when_evidence_is_measured() {
        let mut clock = TurnClock::new(300, 900);
        clock.note_round();
        let first = clock.take_labels();
        assert_eq!(first.len(), 1);
        assert!(first[0].starts_with("Deadline "));
        assert!(first[0].ends_with("0 calls"));
        clock.raise_calls(vec![live("crtsh_certificates")]);
        let second = clock.take_labels();
        assert!(second[0].contains("1 calls") || second[0].contains("1 call"), "{second:?}");
        clock.set_evidence(52_000);
        clock.begin_synthesis();
        let third = clock.label();
        assert!(third.contains("~52k chars evidence"), "{third}");
        assert!(clock.breakdown().contains("synthesis"));
    }
}
