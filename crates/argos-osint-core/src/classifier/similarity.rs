//! Bounded lexical review hints. Similarity is never identity, corroboration or
//! authority to merge a permanent story family or assign a training label.
use super::{ClassifierError, WorkControl};
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pair {
    pub left: String,
    pub right: String,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Similarity {
    pub dice: f64,
    pub jaro_winkler: f64,
}
/// Scores a small index-retrieved reranking set using pinned Natural. Inputs
/// are passed as function arguments; fixed code alone is evaluated as source.
pub fn compare(pairs: &[Pair], control: &WorkControl) -> Result<Vec<Similarity>, ClassifierError> {
    if pairs.len() > 4096
        || pairs
            .iter()
            .any(|pair| pair.left.len() > 2048 || pair.right.len() > 2048)
    {
        return Err(ClassifierError::ModelTooLarge);
    }
    #[cfg(not(feature = "classifier"))]
    {
        let _ = (pairs, control);
        Err(ClassifierError::Unavailable)
    }
    #[cfg(feature = "classifier")]
    {
        use std::sync::atomic::Ordering;
        use std::time::Instant;
        control.check_active()?;
        let _reservation = crate::js_runtime_budget::classifier_process_budget()
            .try_reserve(super::engine::RUNTIME_RESERVATION_BYTES)
            .map_err(|_| ClassifierError::Busy)?;
        let runtime = rquickjs::Runtime::new().map_err(|_| ClassifierError::WorkerStopped)?;
        super::runtime_proof::configure(&runtime);
        let stopped = control.cancel.clone();
        let deadline = (Instant::now() + control.execution_timeout).min(control.overall_deadline);
        runtime.set_interrupt_handler(Some(Box::new(move || {
            stopped.load(Ordering::Acquire) || Instant::now() >= deadline
        })));
        let context =
            rquickjs::Context::full(&runtime).map_err(|_| ClassifierError::WorkerStopped)?;
        context.with(|ctx|->Result<(),ClassifierError> {
            ctx.eval::<(),_>(include_str!("../../assets/classifier/natural-classifier.js")).map_err(|error|super::engine::javascript_error(&ctx,error,"similarity-init"))?;
            ctx.eval::<(),_>(r#"globalThis.argosReviewScore=(input=>JSON.stringify(JSON.parse(input).map(pair=>({dice:pair.left.trim()&&pair.right.trim()?argosNaturalBayes.DiceCoefficient(pair.left,pair.right):0,jaro_winkler:pair.left.trim()&&pair.right.trim()?argosNaturalBayes.JaroWinklerDistance(pair.left,pair.right,{ignoreCase:true}):0}))))"#).map_err(|error|super::engine::javascript_error(&ctx,error,"similarity-init"))?;
            Ok(())
        })?;
        let mut results = Vec::new();
        for batch in pairs.chunks(64) {
            control.check_active()?;
            let stop = control.cancel.clone();
            let deadline =
                (Instant::now() + control.execution_timeout).min(control.overall_deadline);
            runtime.set_interrupt_handler(Some(Box::new(move || {
                stop.load(Ordering::Acquire) || Instant::now() >= deadline
            })));
            let input = serde_json::to_string(batch).map_err(|_| ClassifierError::ModelTooLarge)?;
            let values: Vec<Similarity> =
                context.with(|ctx| -> Result<Vec<Similarity>, ClassifierError> {
                    let score: rquickjs::Function = ctx
                        .globals()
                        .get("argosReviewScore")
                        .map_err(|_| ClassifierError::WorkerStopped)?;
                    let output: String = score.call((input,)).map_err(|error| {
                        super::engine::javascript_error(&ctx, error, "similarity-score")
                    })?;
                    serde_json::from_str(&output).map_err(|_| ClassifierError::WorkerStopped)
                })?;
            control.check_active()?;
            if values.len() != batch.len()
                || values.iter().any(|value| {
                    ![value.dice, value.jaro_winkler]
                        .iter()
                        .all(|n| n.is_finite() && (0.0..=1.0).contains(n))
                })
            {
                return Err(ClassifierError::WorkerStopped);
            }
            results.extend(values);
        }
        Ok(results)
    }
}

#[cfg(all(test, feature = "classifier"))]
mod tests {
    use super::*;
    #[tokio::test]
    async fn pinned_natural_similarity_is_bounded_and_never_assigns_labels() {
        let _serial = crate::classifier::TEST_RUNTIME_LOCK.lock().await;
        let scores = tokio::task::spawn_blocking(|| {
            compare(
                &[
                    Pair {
                        left: "Assembly approves a budget".into(),
                        right: "Assembly approves a budget".into(),
                    },
                    Pair {
                        left: "".into(),
                        right: "".into(),
                    },
                    Pair {
                        left: "Hospital will open on Friday".into(),
                        right: "Hospital will not open on Friday".into(),
                    },
                ],
                &WorkControl::new(),
            )
        })
        .await
        .unwrap()
        .unwrap();
        assert_eq!(
            scores[0],
            Similarity {
                dice: 1.0,
                jaro_winkler: 1.0
            }
        );
        assert_eq!(
            scores[1],
            Similarity {
                dice: 0.0,
                jaro_winkler: 0.0
            }
        );
        assert!(
            scores[2].dice > 0.8 && scores[2].jaro_winkler > 0.9,
            "lexical similarity alone cannot resolve a negated headline"
        );
        let too_many = vec![
            Pair {
                left: "a".into(),
                right: "b".into()
            };
            4097
        ];
        assert!(matches!(
            compare(&too_many, &WorkControl::new()),
            Err(ClassifierError::ModelTooLarge)
        ));
        let control = WorkControl::new();
        control
            .cancel
            .store(true, std::sync::atomic::Ordering::Release);
        assert!(matches!(
            compare(&[], &control),
            Err(ClassifierError::Cancelled)
        ));
    }
}
