//! Neutral process-level memory reservation ledger for JavaScript runtimes.
//!
//! Enforces the process reservation ceiling (512 MiB) across classifier
//! runtimes. Only `std` is imported: this module is neutral and independent
//! of any specific engine or application domain.

use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::sync::atomic::{AtomicUsize, Ordering};

/// Total memory budget (heap + stack) permitted across all active JS runtimes
/// in this process (512 MiB).
pub(crate) const PROCESS_RESERVATION_LIMIT_BYTES: usize = 512 * 1024 * 1024;

/// Process reservation ledger tracking active memory reservations.
#[derive(Debug)]
pub(crate) struct RuntimeBudget {
    limit: usize,
    used: AtomicUsize,
}

/// An active memory reservation guard.
///
/// Dropping this guard releases the reserved bytes synchronously and exactly
/// once from the owning ledger. It is intentionally not `Clone`.
#[derive(Debug)]
#[must_use = "dropping a RuntimeReservation immediately frees the reserved budget"]
pub(crate) struct RuntimeReservation {
    budget: &'static RuntimeBudget,
    bytes: usize,
}

/// Error conditions encountered when requesting a memory reservation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ReservationError {
    /// The requested reservation was invalid (e.g., 0 bytes or exceeds total budget limit).
    InvalidRequest { requested: usize, limit: usize },
    /// The ledger could not accommodate the requested bytes without exceeding its limit.
    Exhausted {
        requested: usize,
        reserved: usize,
        limit: usize,
    },
}

impl Display for ReservationError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRequest { requested, limit } => {
                write!(
                    f,
                    "invalid runtime reservation request of {requested} bytes (limit is {limit} bytes)"
                )
            }
            Self::Exhausted {
                requested,
                reserved,
                limit,
            } => {
                write!(
                    f,
                    "runtime memory budget exhausted: requested {requested} bytes, already reserved {reserved} bytes of {limit} byte limit"
                )
            }
        }
    }
}

impl Error for ReservationError {}

static CLASSIFIER_BUDGET: RuntimeBudget = RuntimeBudget::new(PROCESS_RESERVATION_LIMIT_BYTES);

/// Returns the process-global classifier reservation budget.
pub(crate) fn classifier_process_budget() -> &'static RuntimeBudget {
    &CLASSIFIER_BUDGET
}

impl RuntimeBudget {
    /// Creates a new budget ledger with the given hard byte limit.
    pub(crate) const fn new(limit: usize) -> Self {
        Self {
            limit,
            used: AtomicUsize::new(0),
        }
    }

    /// Attempts to reserve `bytes` from this budget ledger.
    ///
    /// Uses checked addition and atomic compare-and-swap so reservations never
    /// wrap or overbook.
    pub(crate) fn try_reserve(
        &'static self,
        bytes: usize,
    ) -> Result<RuntimeReservation, ReservationError> {
        if bytes == 0 || bytes > self.limit {
            return Err(ReservationError::InvalidRequest {
                requested: bytes,
                limit: self.limit,
            });
        }

        let update_result =
            self.used
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| match current
                    .checked_add(bytes)
                {
                    Some(new_total) if new_total <= self.limit => Some(new_total),
                    _ => None,
                });

        match update_result {
            Ok(_) => Ok(RuntimeReservation {
                budget: self,
                bytes,
            }),
            Err(current_used) => Err(ReservationError::Exhausted {
                requested: bytes,
                reserved: current_used,
                limit: self.limit,
            }),
        }
    }

    /// Returns the currently reserved bytes.
    ///
    /// This value is diagnostic only and must not be used as an authorization
    /// precheck.
    #[must_use]
    pub(crate) fn reserved_bytes(&self) -> usize {
        self.used.load(Ordering::Acquire)
    }

    /// Returns the hard byte limit for this ledger.
    #[must_use]
    pub(crate) const fn limit_bytes(&self) -> usize {
        self.limit
    }
}

impl RuntimeReservation {
    /// Returns the number of bytes held by this reservation.
    #[must_use]
    pub(crate) const fn bytes(&self) -> usize {
        self.bytes
    }
}

impl Drop for RuntimeReservation {
    fn drop(&mut self) {
        self.budget.used.fetch_sub(self.bytes, Ordering::AcqRel);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_budget(limit: usize) -> &'static RuntimeBudget {
        Box::leak(Box::new(RuntimeBudget::new(limit)))
    }

    #[test]
    fn test_zero_bytes_rejected() {
        let budget = create_test_budget(1000);
        let err = budget.try_reserve(0).unwrap_err();
        assert_eq!(
            err,
            ReservationError::InvalidRequest {
                requested: 0,
                limit: 1000
            }
        );
    }

    #[test]
    fn test_exceeding_limit_in_single_request_rejected() {
        let budget = create_test_budget(1000);
        let err = budget.try_reserve(1001).unwrap_err();
        assert_eq!(
            err,
            ReservationError::InvalidRequest {
                requested: 1001,
                limit: 1000
            }
        );
    }

    #[test]
    fn test_reserve_and_release_via_drop() {
        let budget = create_test_budget(1000);
        assert_eq!(budget.reserved_bytes(), 0);

        let res1 = budget
            .try_reserve(400)
            .expect("reservation 1 should succeed");
        assert_eq!(res1.bytes(), 400);
        assert_eq!(budget.reserved_bytes(), 400);

        {
            let res2 = budget
                .try_reserve(500)
                .expect("reservation 2 should succeed");
            assert_eq!(res2.bytes(), 500);
            assert_eq!(budget.reserved_bytes(), 900);

            let err = budget.try_reserve(200).unwrap_err();
            assert_eq!(
                err,
                ReservationError::Exhausted {
                    requested: 200,
                    reserved: 900,
                    limit: 1000
                }
            );
        }

        // res2 dropped, so reserved_bytes should return to 400
        assert_eq!(budget.reserved_bytes(), 400);

        drop(res1);
        assert_eq!(budget.reserved_bytes(), 0);
    }

    #[test]
    fn test_three_runtime_capacity_fits_and_fourth_fails() {
        // Single runtime reservation: 128 MiB + 1 MiB = 135,266,304 bytes
        const RUNTIME_BYTES: usize = 128 * 1024 * 1024 + 1024 * 1024;
        let budget = create_test_budget(PROCESS_RESERVATION_LIMIT_BYTES);

        let r1 = budget.try_reserve(RUNTIME_BYTES).expect("runtime 1 fits");
        let r2 = budget.try_reserve(RUNTIME_BYTES).expect("runtime 2 fits");
        let r3 = budget.try_reserve(RUNTIME_BYTES).expect("runtime 3 fits");

        // Total reserved: 3 * 135,266,304 = 405,798,912 <= 536,870,912
        assert_eq!(budget.reserved_bytes(), 405_798_912);

        // Fourth runtime would require 541,065,216 > 536,870,912 -> fails
        let err = budget.try_reserve(RUNTIME_BYTES).unwrap_err();
        match err {
            ReservationError::Exhausted {
                requested,
                reserved,
                limit,
            } => {
                assert_eq!(requested, RUNTIME_BYTES);
                assert_eq!(reserved, 405_798_912);
                assert_eq!(limit, PROCESS_RESERVATION_LIMIT_BYTES);
            }
            other => panic!("expected Exhausted error, got {other:?}"),
        }

        drop(r1);
        drop(r2);
        drop(r3);
        assert_eq!(budget.reserved_bytes(), 0);
    }

    #[test]
    fn test_boundary_conditions_exact_limit_and_overflow() {
        let budget = create_test_budget(PROCESS_RESERVATION_LIMIT_BYTES);

        // Limit + 1 must fail with InvalidRequest
        let err_plus_one = budget
            .try_reserve(PROCESS_RESERVATION_LIMIT_BYTES + 1)
            .unwrap_err();
        assert_eq!(
            err_plus_one,
            ReservationError::InvalidRequest {
                requested: PROCESS_RESERVATION_LIMIT_BYTES + 1,
                limit: PROCESS_RESERVATION_LIMIT_BYTES,
            }
        );

        // usize::MAX must fail with InvalidRequest
        let err_max = budget.try_reserve(usize::MAX).unwrap_err();
        assert_eq!(
            err_max,
            ReservationError::InvalidRequest {
                requested: usize::MAX,
                limit: PROCESS_RESERVATION_LIMIT_BYTES,
            }
        );

        // Exactly the limit should succeed
        let res = budget
            .try_reserve(PROCESS_RESERVATION_LIMIT_BYTES)
            .expect("reserving exactly the budget limit should succeed");
        assert_eq!(budget.reserved_bytes(), PROCESS_RESERVATION_LIMIT_BYTES);

        // Once at full capacity, reserving 1 byte must fail with Exhausted
        let err_exhausted = budget.try_reserve(1).unwrap_err();
        assert_eq!(
            err_exhausted,
            ReservationError::Exhausted {
                requested: 1,
                reserved: PROCESS_RESERVATION_LIMIT_BYTES,
                limit: PROCESS_RESERVATION_LIMIT_BYTES,
            }
        );

        // Dropping must restore 0 bytes
        drop(res);
        assert_eq!(budget.reserved_bytes(), 0);
    }

    #[test]
    fn test_concurrency_race_conditions_and_no_overbooking() {
        use std::sync::{Arc, Barrier};
        use std::thread;

        const CAPACITY: usize = 1_000_000;
        const REQUEST_SIZE: usize = 50_000;
        const NUM_THREADS: usize = 32;

        let budget = create_test_budget(CAPACITY);
        let barrier = Arc::new(Barrier::new(NUM_THREADS));

        let handles: Vec<_> = (0..NUM_THREADS)
            .map(|_| {
                let c_barrier = barrier.clone();
                thread::spawn(move || {
                    c_barrier.wait();
                    budget.try_reserve(REQUEST_SIZE)
                })
            })
            .collect();

        let mut successes = Vec::new();
        let mut failures = Vec::new();

        for handle in handles {
            match handle.join().expect("thread should not panic") {
                Ok(res) => successes.push(res),
                Err(err) => failures.push(err),
            }
        }

        assert_eq!(
            successes.len(),
            20,
            "exactly 20 reservations should succeed"
        );
        assert_eq!(failures.len(), 12, "exactly 12 reservations should fail");
        assert_eq!(
            budget.reserved_bytes(),
            CAPACITY,
            "budget should be exactly at full capacity"
        );

        for err in failures {
            match err {
                ReservationError::Exhausted {
                    requested, limit, ..
                } => {
                    assert_eq!(requested, REQUEST_SIZE);
                    assert_eq!(limit, CAPACITY);
                }
                other => panic!("expected Exhausted error, got {other:?}"),
            }
        }

        // Dropping all reservations concurrently
        let drop_barrier = Arc::new(Barrier::new(successes.len()));
        let drop_handles: Vec<_> = successes
            .into_iter()
            .map(|res| {
                let d_barrier = drop_barrier.clone();
                thread::spawn(move || {
                    d_barrier.wait();
                    drop(res);
                })
            })
            .collect();

        for handle in drop_handles {
            handle.join().expect("thread join should succeed");
        }

        assert_eq!(
            budget.reserved_bytes(),
            0,
            "budget should return exactly to 0 after all drops"
        );
    }

    #[test]
    fn test_concurrency_high_churn_stress() {
        use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
        use std::sync::Arc;
        use std::thread;

        const CAPACITY: usize = 200_000;
        const REQUEST_SIZE: usize = 25_000;
        const NUM_THREADS: usize = 16;
        const ITERS: usize = 200;

        let budget = create_test_budget(CAPACITY);
        let total_reservations = Arc::new(AtomicUsize::new(0));

        let handles: Vec<_> = (0..NUM_THREADS)
            .map(|_| {
                let total_res = total_reservations.clone();
                thread::spawn(move || {
                    for _ in 0..ITERS {
                        if let Ok(res) = budget.try_reserve(REQUEST_SIZE) {
                            total_res.fetch_add(1, AtomicOrdering::Relaxed);
                            assert!(
                                budget.reserved_bytes() <= CAPACITY,
                                "reserved_bytes must never exceed capacity"
                            );
                            thread::yield_now();
                            drop(res);
                        }
                    }
                })
            })
            .collect();

        for handle in handles {
            handle.join().expect("thread join should succeed");
        }

        assert!(
            total_reservations.load(AtomicOrdering::Relaxed) > 0,
            "at least some reservations should have succeeded"
        );
        assert_eq!(
            budget.reserved_bytes(),
            0,
            "budget must be fully released after churn test"
        );
    }

    #[test]
    fn test_drop_synchronous_recovery_and_rebooking() {
        const RUNTIME_BYTES: usize = 128 * 1024 * 1024 + 1024 * 1024;
        let budget = create_test_budget(PROCESS_RESERVATION_LIMIT_BYTES);

        let r1 = budget.try_reserve(RUNTIME_BYTES).unwrap();
        let r2 = budget.try_reserve(RUNTIME_BYTES).unwrap();
        let r3 = budget.try_reserve(RUNTIME_BYTES).unwrap();

        // 4th fails
        let err = budget.try_reserve(RUNTIME_BYTES).unwrap_err();
        assert!(matches!(err, ReservationError::Exhausted { .. }));

        // Drop r2: immediately restores capacity
        drop(r2);
        assert_eq!(budget.reserved_bytes(), 2 * RUNTIME_BYTES);

        // 4th now succeeds immediately!
        let r4 = budget
            .try_reserve(RUNTIME_BYTES)
            .expect("rebooking must succeed after drop");
        assert_eq!(budget.reserved_bytes(), 3 * RUNTIME_BYTES);

        drop(r1);
        drop(r3);
        drop(r4);
        assert_eq!(budget.reserved_bytes(), 0);
    }

    #[test]
    fn test_drop_on_panic_unwinding() {
        let budget = create_test_budget(10_000);

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _res = budget.try_reserve(5_000).unwrap();
            assert_eq!(budget.reserved_bytes(), 5_000);
            panic!("deliberate panic to test drop during unwinding");
        }));

        assert!(result.is_err(), "thread should have panicked");
        assert_eq!(
            budget.reserved_bytes(),
            0,
            "reservation should be dropped and reclaimed during panic unwinding"
        );
    }
}
