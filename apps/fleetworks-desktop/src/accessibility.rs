//! Native display preferences, sampled only on the UI thread. Fixture callers
//! supply explicit values and never read or modify the machine's preferences.

use std::cell::Cell;
use std::time::Duration;
use std::time::Instant;

use objc2_app_kit::NSWorkspace;

const SAMPLE_INTERVAL: Duration = Duration::from_secs(1);

/// Developer-only appearance inputs; the production CLI exposes no override.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct FixtureAccessibility {
    pub reduced_motion: bool,
    pub reduced_transparency: bool,
    pub matte_fallback: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct ViewPreferences {
    pub reduced_motion: bool,
    pub reduced_transparency: bool,
    pub matte_fallback: bool,
}

impl ViewPreferences {
    pub(crate) fn matte(self) -> bool {
        self.reduced_transparency || self.matte_fallback
    }
}

/// One per-window cache. Focus invalidates it; redraws coalesce OS reads without
/// scheduling decorative work or creating a second preference store.
#[derive(Default)]
pub struct Accessibility {
    pub fixture_override: Option<FixtureAccessibility>,
    transparency: Cell<Option<(Instant, bool)>>,
}

impl Accessibility {
    pub(crate) fn invalidate(&self) {
        self.transparency.set(None);
    }

    pub(crate) fn for_view(&self, fixture: bool, rune_reduced_motion: bool) -> ViewPreferences {
        self.resolve(fixture, rune_reduced_motion, Instant::now(), || {
            NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceTransparency()
        })
    }

    fn resolve(
        &self,
        fixture: bool,
        rune_reduced_motion: bool,
        now: Instant,
        sample: impl FnOnce() -> bool,
    ) -> ViewPreferences {
        if fixture {
            let input = self.fixture_override.unwrap_or_default();
            return ViewPreferences {
                reduced_motion: rune_reduced_motion || input.reduced_motion,
                reduced_transparency: input.reduced_transparency,
                matte_fallback: input.matte_fallback,
            };
        }
        let reduced_transparency = match self.transparency.get() {
            Some((observed, value))
                if now
                    .checked_duration_since(observed)
                    .is_some_and(|elapsed| elapsed < SAMPLE_INTERVAL) =>
            {
                value
            }
            _ => {
                let value = sample();
                self.transparency.set(Some((now, value)));
                value
            }
        };
        ViewPreferences {
            reduced_motion: rune_reduced_motion,
            reduced_transparency,
            matte_fallback: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixtures_never_sample_os_or_disable_host_reduced_motion() {
        let appearance = Accessibility {
            fixture_override: Some(FixtureAccessibility {
                reduced_motion: false,
                reduced_transparency: true,
                matte_fallback: true,
            }),
            ..Accessibility::default()
        };
        let preferences = appearance.resolve(true, true, Instant::now(), || {
            panic!("fixture must not sample OS")
        });
        assert!(
            preferences.reduced_motion && preferences.reduced_transparency && preferences.matte()
        );
        assert!(
            appearance.transparency.get().is_none(),
            "synthetic values never populate the OS cache"
        );
    }

    #[test]
    fn native_reads_coalesce_until_focus_invalidates_cache() {
        let appearance = Accessibility::default();
        let now = Instant::now();
        assert!(
            !appearance
                .resolve(false, false, now, || false)
                .reduced_transparency
        );
        assert!(
            !appearance
                .resolve(false, false, now + Duration::from_millis(100), || panic!(
                    "coalesced redraw must use cache"
                ))
                .reduced_transparency
        );
        appearance.invalidate();
        assert!(
            appearance
                .resolve(false, false, now + Duration::from_millis(200), || true)
                .reduced_transparency
        );
        assert!(
            !appearance
                .resolve(false, false, now + Duration::from_secs(2), || false)
                .reduced_transparency
        );
    }
}
