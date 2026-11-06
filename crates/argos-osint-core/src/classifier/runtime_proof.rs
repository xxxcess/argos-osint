//! Owned, fixed-code checks of the same limits used by classifier workers.
use super::{ClassifierError, WorkControl, RUNTIME_MEMORY_LIMIT_BYTES, RUNTIME_STACK_LIMIT_BYTES};
use crate::js_runtime_budget::classifier_process_budget;
use std::sync::atomic::Ordering;
use std::thread;
use std::time::Instant;

/// Not deserializable: only actual successful limit checks construct this proof.
pub(crate) struct RuntimeLimitProof {
    reservation: u64,
}
impl RuntimeLimitProof {
    pub(crate) fn stack_limit_bytes(&self) -> u64 {
        RUNTIME_STACK_LIMIT_BYTES as u64
    }
    pub(crate) fn observed_reservation_bytes(&self) -> u64 {
        self.reservation
    }
}
pub(crate) fn configure(runtime: &rquickjs::Runtime) {
    runtime.set_memory_limit(RUNTIME_MEMORY_LIMIT_BYTES);
    runtime.set_max_stack_size(RUNTIME_STACK_LIMIT_BYTES);
}
/// Called from an owned blocking task, never an async executor worker. Each
/// runtime and reservation is dropped on its owning thread before joining.
pub(crate) fn verify(control: &WorkControl) -> Result<RuntimeLimitProof, ClassifierError> {
    let control = control.clone();
    thread::Builder::new().name("classifier-limit-proof".into()).spawn(move || {
        let mut reservation=0;
        for (resource,source) in [
            ("stack", "function recurse(){return 1+recurse()} recurse(); 'unexpected-success'"),
            ("heap", "const chunks=[]; for(let i=0;i<140;i++) chunks.push(new Uint8Array(1024*1024)); 'unexpected-success'"),
        ] {
            control.check_active()?;
            let budget=classifier_process_budget();
            let _reservation=budget.try_reserve(super::engine::RUNTIME_RESERVATION_BYTES).map_err(|_|ClassifierError::Busy)?;
            reservation=reservation.max(budget.reserved_bytes() as u64);
            let runtime=rquickjs::Runtime::new().map_err(|_|ClassifierError::WorkerStopped)?;
            configure(&runtime);
            let stop=control.cancel.clone();
            let deadline=(Instant::now()+control.execution_timeout).min(control.overall_deadline);
            runtime.set_interrupt_handler(Some(Box::new(move ||stop.load(Ordering::Acquire)||Instant::now()>=deadline)));
            let context=rquickjs::Context::full(&runtime).map_err(|_|ClassifierError::WorkerStopped)?;
            let result=context.with(|ctx|ctx.eval::<String,_>(source).map_err(|error|super::engine::javascript_error(&ctx,error,"runtime-limit-proof")));
            control.check_active()?;
            if !matches!(result,Err(ClassifierError::RuntimeLimitExceeded {resource:ref found}) if found==resource) {
                return Err(ClassifierError::InvalidArtifact(format!("runtime_{resource}_proof_failed")));
            }
        }
        Ok(RuntimeLimitProof {reservation})
    }).map_err(|_|ClassifierError::WorkerStopped)?.join().map_err(|_|ClassifierError::WorkerStopped)?
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn actual_native_limits_produce_proof_and_cancel_does_not() {
        let _serial = crate::classifier::TEST_RUNTIME_LOCK.lock().await;
        let proof = tokio::task::spawn_blocking(|| verify(&WorkControl::new()))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(proof.stack_limit_bytes(), 1024 * 1024);
        assert!(
            proof.observed_reservation_bytes()
                >= super::super::engine::RUNTIME_RESERVATION_BYTES as u64
        );
        let control = WorkControl::new();
        control.cancel.store(true, Ordering::Release);
        assert!(matches!(
            tokio::task::spawn_blocking(move || verify(&control))
                .await
                .unwrap(),
            Err(ClassifierError::Cancelled)
        ));
    }
}
