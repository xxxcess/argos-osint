//! Bounded numeric telemetry capture for an isolated fixed-cohort benchmark.
use serde_json::{Map, Value};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc, Mutex,
};
use tracing::{
    span::{Attributes, Id, Record},
    Event, Metadata, Subscriber,
};
#[derive(Clone, Default)]
pub struct Measure {
    pub events: Arc<Mutex<Vec<Value>>>,
    pub dropped: Arc<AtomicU64>,
    next: Arc<AtomicU64>,
}
struct Fields(Map<String, Value>);
impl tracing::field::Visit for Fields {
    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        self.0.insert(field.name().into(), value.into());
    }
    fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
        self.0.insert(field.name().into(), value.into());
    }
    fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
        self.0.insert(field.name().into(), value.into());
    }
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        self.0.insert(
            field.name().into(),
            Value::String(value.chars().take(256).collect()),
        );
    }
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        self.record_str(field, &format!("{value:?}"));
    }
}
impl Subscriber for Measure {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.target() == "argos::atlas_derivatives"
    }
    fn new_span(&self, _: &Attributes<'_>) -> Id {
        Id::from_u64(self.next.fetch_add(1, Ordering::Relaxed) + 1)
    }
    fn record(&self, _: &Id, _: &Record<'_>) {}
    fn record_follows_from(&self, _: &Id, _: &Id) {}
    fn event(&self, event: &Event<'_>) {
        if !self.enabled(event.metadata()) {
            return;
        }
        let mut fields = Fields(Map::new());
        event.record(&mut fields);
        let mut events = self.events.lock().unwrap();
        if events.len() < 8192 {
            events.push(Value::Object(fields.0));
        } else {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }
    fn enter(&self, _: &Id) {}
    fn exit(&self, _: &Id) {}
}
