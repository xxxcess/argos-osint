//! Provider retry limits are deterministic and bounded for untrusted headers.
use argos_osint_core::provider_request::retry_after_delay;
use proptest::prelude::*;
use std::time::Duration;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]
    #[test]
    fn retry_after_numeric_seconds_are_capped(seconds in any::<u64>()) {
        let raw = format!("  {seconds}  ");
        prop_assert_eq!(retry_after_delay(Some(&raw)), Some(Duration::from_secs(seconds.min(300))));
    }

    #[test]
    fn arbitrary_retry_after_never_exceeds_policy(raw in any::<String>()) {
        if let Some(delay) = retry_after_delay(Some(&raw)) {
            prop_assert!(delay <= Duration::from_secs(300));
            prop_assert!(raw.trim().parse::<u64>().is_ok());
        }
    }
}

#[test]
fn deadline_profiles_keep_transport_and_model_waits_within_total() {
    use argos_osint_core::provider_attempt::Deadlines;
    for deadlines in [
        Deadlines::ordinary(),
        Deadlines::insight_stream(),
        Deadlines::insight_non_stream(),
        Deadlines::insight_stream_thinking(),
        Deadlines::insight_non_stream_thinking(),
        Deadlines::summarization(),
        Deadlines::native_decision(),
    ] {
        assert!(!deadlines.total.is_zero());
        assert!(deadlines.connect <= deadlines.first_response);
        assert!(deadlines.first_response <= deadlines.total);
        assert!(deadlines.idle <= deadlines.total);
        assert!(deadlines
            .first_output
            .is_none_or(|wait| wait <= deadlines.total));
        assert!(deadlines
            .model_inactivity
            .is_none_or(|wait| wait <= deadlines.total));
    }
    assert_eq!(Deadlines::native_decision(), Deadlines::ordinary());
    let standard = Deadlines::insight_stream();
    let thinking = Deadlines::insight_stream_thinking();
    assert_eq!(standard.total, thinking.total);
    assert!(thinking.first_output > standard.first_output);
    assert!(thinking.model_inactivity > standard.model_inactivity);
}
