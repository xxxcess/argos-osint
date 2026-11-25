//! Compare one verified reading with the preceding reading of the same subject.

use argos_osint_core::storage::inbox_qa::Observation;
use argos_osint_core::storage::inbox_qa::ObservationValue;
use argos_osint_core::storage::inbox_qa::Source;
use chrono::DateTime;
use fleetworks_contracts::chat::QaMeasurement;
use fleetworks_contracts::chat::QaMeasurementUnit;
use fleetworks_contracts::chat::QA_MAX_WORDS;
use fleetworks_contracts::qa_worker::QaPassage;

pub(super) fn single_observation(sources: &[Source], state: Option<&str>) -> Option<Observation> {
    let [source] = sources else {
        return None;
    };
    let value = if let Some(state) = state {
        ObservationValue::State(state.to_owned())
    } else {
        ObservationValue::Measurement(source.measurement.clone()?)
    };
    Some(Observation {
        source: source.reference.clone(),
        observed_at: source.observed_at.clone(),
        value,
    })
}

/// Add the previous citation after the fresh finding, within the same reply budget.
pub(super) fn append(
    previous: &Observation,
    current: &Observation,
    passages: &mut Vec<QaPassage>,
    sources: &mut Vec<Source>,
) -> bool {
    let Some((text, measurement)) = describe(previous, current) else {
        return false;
    };
    let current_words: usize = passages
        .iter()
        .map(|passage| passage.text.split_whitespace().count())
        .sum();
    if current_words + text.split_whitespace().count() > QA_MAX_WORDS {
        return false;
    }
    let opaque_id = format!("passage-{}", passages.len());
    passages.push(QaPassage::new(opaque_id.clone(), text));
    sources.push(Source {
        opaque_id,
        reference: previous.source.clone(),
        label: "Previous observation".into(),
        observed_at: previous.observed_at.clone(),
        measurement,
        whole_passage: false,
    });
    true
}

fn describe(
    previous: &Observation,
    current: &Observation,
) -> Option<(String, Option<QaMeasurement>)> {
    if previous.source != current.source
        || DateTime::parse_from_rfc3339(&previous.observed_at).ok()?
            >= DateTime::parse_from_rfc3339(&current.observed_at).ok()?
    {
        return None;
    }
    let (before, now, measurement) = match (&previous.value, &current.value) {
        (ObservationValue::State(before), ObservationValue::State(now)) => {
            (before.clone(), now.clone(), None)
        }
        (ObservationValue::Measurement(before), ObservationValue::Measurement(now))
            if before.unit == now.unit && before.subject == now.subject =>
        {
            (
                format_measurement(before)?,
                format_measurement(now)?,
                Some(before.clone()),
            )
        }
        _ => return None,
    };
    let text = if before == now {
        format!("The previous observation was also {before}.")
    } else {
        format!("The previous observation was {before}.")
    };
    Some((text, measurement))
}

fn format_measurement(measurement: &QaMeasurement) -> Option<String> {
    let value = measurement.value;
    Some(match measurement.unit {
        QaMeasurementUnit::PercentTenths => format!("{}.{}%", value / 10, value % 10),
        QaMeasurementUnit::Jobs if value == 1 => "1 job".to_owned(),
        QaMeasurementUnit::Jobs => format!("{value} jobs"),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use fleetworks_contracts::chat::ChatSourceKind;
    use fleetworks_contracts::chat::ChatSourceReference;

    use super::*;

    fn job(id: &str, at: &str, state: &str) -> Observation {
        Observation {
            source: ChatSourceReference::new(ChatSourceKind::Job, id.into()).unwrap(),
            observed_at: at.into(),
            value: ObservationValue::State(state.into()),
        }
    }

    #[test]
    fn compares_two_successive_rechecks_using_the_immediately_preceding_state() {
        let initial = job("index", "2026-11-24T10:00:00Z", "running");
        let current = job("index", "2026-11-24T10:00:01Z", "completed");
        let repeated = job("index", "2026-11-24T10:00:02Z", "completed");
        assert_eq!(
            describe(&initial, &current).unwrap().0,
            "The previous observation was running."
        );
        assert_eq!(
            describe(&current, &repeated).unwrap().0,
            "The previous observation was also completed."
        );
        assert!(describe(&current, &current).is_none());
        assert!(describe(&current, &initial).is_none());
        let other = job("seed", &repeated.observed_at, "completed");
        assert!(describe(&current, &other).is_none());
    }

    #[test]
    fn requires_matching_measurement_subject_and_unit() {
        let source =
            ChatSourceReference::new(ChatSourceKind::JobCount, "failed_last_24_hours".into())
                .unwrap();
        let measurement = |at: &str, value, unit, subject: &str| Observation {
            source: source.clone(),
            observed_at: at.into(),
            value: ObservationValue::Measurement(QaMeasurement::new(value, unit, subject.into())),
        };
        let start = "2026-11-24T10:00:00Z";
        let later = "2026-11-24T10:00:01Z";
        let initial = measurement(
            start,
            1,
            QaMeasurementUnit::Jobs,
            "Failed jobs in the last 24 hours",
        );
        let current = measurement(
            later,
            2,
            QaMeasurementUnit::Jobs,
            "Failed jobs in the last 24 hours",
        );
        assert_eq!(
            describe(&initial, &current).unwrap().0,
            "The previous observation was 1 job."
        );
        assert!(describe(
            &initial,
            &measurement(later, 2, QaMeasurementUnit::Jobs, "Failed jobs this week")
        )
        .is_none());
        assert!(describe(
            &initial,
            &measurement(
                later,
                2,
                QaMeasurementUnit::PercentTenths,
                "Failed jobs in the last 24 hours"
            )
        )
        .is_none());
    }
}
