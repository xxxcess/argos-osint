//! Bounded independent execution lanes behind one durable queue/relay. A
//! blocked bulk lane cannot hold another lane's execution future or channel.

use argos_osint_core::osint::manual;
use argos_osint_core::storage::classifier_activation;
use argos_osint_core::storage::setup_jobs;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::sync::OwnedSemaphorePermit;
use tokio::sync::Semaphore;

/// One owned delivery/future per process and resource pool. A relay never
/// parks an unrenewed queue lease behind a long-running operation.
/// Database admission/claim fences remain the cross-replica authority.
pub const CAPACITY_PER_POOL: usize = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Lane {
    Tools,
    Seed,
    Train,
    Activate,
    Reindex,
}

impl Lane {
    pub const ALL: [Self; 5] = [
        Self::Tools,
        Self::Seed,
        Self::Train,
        Self::Activate,
        Self::Reindex,
    ];

    pub fn pool(self) -> &'static str {
        match self {
            Self::Tools => manual::TOOLS_POOL,
            Self::Seed => setup_jobs::SEED_POOL,
            Self::Train => setup_jobs::TRAIN_POOL,
            Self::Activate => classifier_activation::POOL,
            Self::Reindex => setup_jobs::REINDEX_POOL,
        }
    }
}

pub enum RouteError<T> {
    Full(T),
    Unknown(T),
}

pub struct Routes<T> {
    lanes: Vec<(Lane, mpsc::Sender<Routed<T>>, Arc<Semaphore>)>,
    bulk: Arc<Semaphore>,
}

pub struct Routed<T> {
    value: T,
    permit: OwnedSemaphorePermit,
    bulk: Option<OwnedSemaphorePermit>,
}
impl<T> Routed<T> {
    pub fn into_parts(self) -> (T, (OwnedSemaphorePermit, Option<OwnedSemaphorePermit>)) {
        (self.value, (self.permit, self.bulk))
    }
}

/// A resource lane paired with its bounded delivery receiver.
pub type LaneReceiver<T> = (Lane, mpsc::Receiver<Routed<T>>);

pub fn bounded<T>(reindex: bool) -> (Routes<T>, Vec<LaneReceiver<T>>) {
    let mut senders = Vec::new();
    let mut receivers = Vec::new();
    for lane in Lane::ALL {
        if lane == Lane::Reindex && !reindex {
            continue;
        }
        let (sender, receiver) = mpsc::channel(CAPACITY_PER_POOL);
        senders.push((lane, sender, Arc::new(Semaphore::new(CAPACITY_PER_POOL))));
        receivers.push((lane, receiver));
    }
    (
        Routes {
            lanes: senders,
            bulk: Arc::new(Semaphore::new(1)),
        },
        receivers,
    )
}

impl<T> Routes<T> {
    pub fn try_send(&self, pool: &str, value: T) -> Result<(), RouteError<T>> {
        let Some((lane, sender, capacity)) =
            self.lanes.iter().find(|(lane, _, _)| lane.pool() == pool)
        else {
            return Err(RouteError::Unknown(value));
        };
        let Ok(permit) = Arc::clone(capacity).try_acquire_owned() else {
            return Err(RouteError::Full(value));
        };
        // Seed, Natural training and activation share the bulk-memory lane;
        // neither can hold the independent interactive Tools slot.
        let bulk = if matches!(lane, Lane::Seed | Lane::Train | Lane::Activate) {
            match Arc::clone(&self.bulk).try_acquire_owned() {
                Ok(permit) => Some(permit),
                Err(_) => return Err(RouteError::Full(value)),
            }
        } else {
            None
        };
        sender
            .try_send(Routed {
                value,
                permit,
                bulk,
            })
            .map_err(|error| RouteError::Full(error.into_inner().value))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn blocked_bulk_lane_does_not_block_interactive_or_other_resource_work() {
        let (routes, mut receivers) = bounded::<u8>(true);
        let (_, mut seed) = receivers.remove(1);
        let (_, mut tools) = receivers.remove(0);
        assert!(routes.try_send(setup_jobs::SEED_POOL, 1).is_ok());
        assert!(matches!(
            routes.try_send(setup_jobs::TRAIN_POOL, 4),
            Err(RouteError::Full(4))
        ));
        assert!(matches!(
            routes.try_send(classifier_activation::POOL, 5),
            Err(RouteError::Full(5))
        ));
        assert!(matches!(
            routes.try_send(setup_jobs::SEED_POOL, 2),
            Err(RouteError::Full(2))
        ));
        assert!(routes.try_send(manual::TOOLS_POOL, 3).is_ok());
        let (value, tool_slot) = tools.recv().await.expect("interactive route").into_parts();
        assert_eq!(value, 3);
        drop(tool_slot);
        let (value, seed_slot) = seed.recv().await.expect("bulk route").into_parts();
        assert_eq!(value, 1);
        assert!(matches!(
            routes.try_send(setup_jobs::SEED_POOL, 2),
            Err(RouteError::Full(2))
        ));
        drop(seed_slot);
        assert!(routes.try_send(setup_jobs::SEED_POOL, 2).is_ok());
    }

    #[test]
    fn unconfigured_reindex_is_not_routed_or_silently_executed() {
        let (routes, _) = bounded::<u8>(false);
        assert!(matches!(
            routes.try_send(setup_jobs::REINDEX_POOL, 1),
            Err(RouteError::Unknown(1))
        ));
    }
}
