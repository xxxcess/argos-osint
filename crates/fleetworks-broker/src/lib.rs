//! RabbitMQ transport for Fleetworks dispatch identifiers.
//!
//! The caller owns durable job state and retry decisions. A failed publish may
//! have reached the broker, so a retried outbox record must be idempotent.

use std::fmt;
use std::future::Future;
use std::time::Duration;

use futures_util::StreamExt;
use lapin::message::Delivery;
use lapin::options::BasicAckOptions;
use lapin::options::BasicConsumeOptions;
use lapin::options::BasicNackOptions;
use lapin::options::BasicPublishOptions;
use lapin::options::BasicQosOptions;
use lapin::options::ConfirmSelectOptions;
use lapin::options::QueueDeclareOptions;
use lapin::types::AMQPValue;
use lapin::types::FieldTable;
use lapin::types::ShortString;
use lapin::BasicProperties;
use lapin::Channel;
use lapin::Confirmation;
use lapin::Connection;
use lapin::ConnectionProperties;

const RPC_TIMEOUT: Duration = Duration::from_secs(15);
const DEAD_SUFFIX: &str = ".dead";
const MAX_QUEUE_BYTES: usize = 200;

/// Maximum size of an encoded dispatch envelope.
pub const MAX_PAYLOAD_BYTES: usize = 16 * 1024;

/// Maximum unacknowledged delivery count per consumer.
pub const MAX_PREFETCH: u16 = 32;

/// Transport failures expose operation names without broker replies or URLs.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum BrokerError {
    InvalidEndpoint,
    InvalidQueue,
    InvalidPrefetch,
    InvalidPayload,
    TimedOut(&'static str),
    Protocol(&'static str),
    Unroutable,
    Unconfirmed,
    Unsettled,
}

impl fmt::Display for BrokerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidEndpoint => f.write_str("expected an authenticated AMQP or AMQPS URL"),
            Self::InvalidQueue => f.write_str("invalid dispatch queue name"),
            Self::InvalidPrefetch => write!(f, "dispatch prefetch must be in 1..={MAX_PREFETCH}"),
            Self::InvalidPayload => write!(
                f,
                "dispatch payload must be in 1..={MAX_PAYLOAD_BYTES} bytes"
            ),
            Self::TimedOut(operation) => write!(f, "broker {operation} timed out"),
            Self::Protocol(operation) => write!(f, "broker {operation} failed"),
            Self::Unroutable => f.write_str("broker returned an unroutable dispatch"),
            Self::Unconfirmed => f.write_str("broker did not confirm dispatch publication"),
            Self::Unsettled => f.write_str("broker delivery could not be settled"),
        }
    }
}

impl std::error::Error for BrokerError {}

type Result<T> = std::result::Result<T, BrokerError>;

/// Connection and confirm-enabled publish channel for one dispatch queue.
pub struct Broker {
    connection: Connection,
    publish: Channel,
    queue: ShortString,
}

impl Broker {
    /// Open the broker and declare durable dispatch and dead-letter queues.
    ///
    /// Use a stable, workspace-scoped queue name. A failed connection or
    /// channel operation can be recovered by opening a fresh broker session.
    pub async fn connect(endpoint: &str, queue: &str) -> Result<Self> {
        validate_endpoint(endpoint)?;
        validate_queue(queue)?;
        let connection = rpc(
            "connect",
            Connection::connect(endpoint, ConnectionProperties::default()),
        )
        .await?;
        let publish = rpc("open publisher", connection.create_channel()).await?;
        let dead_queue = format!("{queue}{DEAD_SUFFIX}");
        rpc(
            "declare dead-letter queue",
            publish.queue_declare(
                dead_queue.as_str().into(),
                QueueDeclareOptions::durable(),
                FieldTable::default(),
            ),
        )
        .await?;

        // The default exchange routes by queue name. Give dead letters their
        // own routing key so rejects cannot return to the dispatch queue.
        let mut arguments = FieldTable::default();
        arguments.insert(
            "x-dead-letter-exchange".into(),
            AMQPValue::LongString("".into()),
        );
        arguments.insert(
            "x-dead-letter-routing-key".into(),
            AMQPValue::LongString(dead_queue.as_str().into()),
        );
        let queue: ShortString = queue.into();
        rpc(
            "declare dispatch queue",
            publish.queue_declare(queue.clone(), QueueDeclareOptions::durable(), arguments),
        )
        .await?;
        rpc(
            "enable publisher confirms",
            publish.confirm_select(ConfirmSelectOptions::default()),
        )
        .await?;

        Ok(Self {
            connection,
            publish,
            queue,
        })
    }

    /// Publish a persistent dispatch and wait for a successful routed confirm.
    pub async fn publish(&self, payload: &[u8]) -> Result<()> {
        validate_payload(payload)?;
        let confirmation = rpc("publish", async {
            self.publish
                .basic_publish(
                    "".into(),
                    self.queue.clone(),
                    BasicPublishOptions {
                        mandatory: true,
                        ..BasicPublishOptions::default()
                    },
                    payload,
                    BasicProperties::default()
                        .with_delivery_mode(2)
                        .with_content_type("application/json".into()),
                )
                .await?
                .await
        })
        .await?;
        match confirmation {
            Confirmation::Ack(None) => Ok(()),
            Confirmation::Ack(Some(_)) | Confirmation::Nack(Some(_)) => {
                Err(BrokerError::Unroutable)
            }
            Confirmation::Nack(None) | Confirmation::NotRequested => Err(BrokerError::Unconfirmed),
        }
    }

    /// Subscribe using a separate channel and bounded manual acknowledgements.
    pub async fn consumer(&self, prefetch: u16) -> Result<Consumer> {
        if !(1..=MAX_PREFETCH).contains(&prefetch) {
            return Err(BrokerError::InvalidPrefetch);
        }
        let channel = rpc("open consumer", self.connection.create_channel()).await?;
        rpc(
            "set prefetch",
            channel.basic_qos(prefetch, BasicQosOptions::default()),
        )
        .await?;
        let inner = rpc(
            "subscribe",
            channel.basic_consume(
                self.queue.clone(),
                "".into(),
                BasicConsumeOptions::default(),
                FieldTable::default(),
            ),
        )
        .await?;
        Ok(Consumer {
            inner,
            _channel: channel,
        })
    }
}

/// One manually acknowledged dispatch subscription.
pub struct Consumer {
    inner: lapin::Consumer,
    _channel: Channel,
}

impl Consumer {
    /// Wait for the next delivery or report that the subscription ended.
    ///
    /// An invalid payload is dead-lettered before this returns an error.
    pub async fn next(&mut self) -> Result<Option<Received>> {
        let Some(delivery) = self.inner.next().await else {
            return Ok(None);
        };
        let received = Received(delivery.map_err(|_| BrokerError::Protocol("receive"))?);
        if let Err(error) = validate_payload(received.payload()) {
            received.settle(Disposition::DeadLetter).await?;
            return Err(error);
        }
        Ok(Some(received))
    }
}

/// How to settle a delivery after the caller decides its durable outcome.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Disposition {
    Ack,
    Requeue,
    DeadLetter,
}

/// A dispatch envelope held for a single explicit acknowledgement decision.
#[must_use = "settle the delivery after handling its durable state"]
pub struct Received(Delivery);

impl Received {
    /// Return the encoded dispatch envelope.
    pub fn payload(&self) -> &[u8] {
        &self.0.data
    }

    /// Settle this delivery once. Ack only after durable state has committed.
    pub async fn settle(self, disposition: Disposition) -> Result<()> {
        let settled = rpc("settle delivery", async {
            match disposition {
                Disposition::Ack => self.0.ack(BasicAckOptions::default()).await,
                Disposition::Requeue | Disposition::DeadLetter => {
                    self.0
                        .nack(BasicNackOptions {
                            requeue: disposition == Disposition::Requeue,
                            ..BasicNackOptions::default()
                        })
                        .await
                }
            }
        })
        .await?;
        if settled {
            Ok(())
        } else {
            Err(BrokerError::Unsettled)
        }
    }
}

async fn rpc<T>(
    operation: &'static str,
    future: impl Future<Output = lapin::Result<T>>,
) -> Result<T> {
    tokio::time::timeout(RPC_TIMEOUT, future)
        .await
        .map_err(|_| BrokerError::TimedOut(operation))?
        .map_err(|_| BrokerError::Protocol(operation))
}

fn validate_endpoint(endpoint: &str) -> Result<()> {
    let parsed = url::Url::parse(endpoint).map_err(|_| BrokerError::InvalidEndpoint)?;
    if !matches!(parsed.scheme(), "amqp" | "amqps")
        || parsed.host_str().is_none()
        || parsed.username().is_empty()
        || parsed.password().is_none_or(|password| password.is_empty())
        || parsed.fragment().is_some()
        || endpoint.chars().any(char::is_whitespace)
        || endpoint.chars().any(char::is_control)
    {
        return Err(BrokerError::InvalidEndpoint);
    }
    Ok(())
}

fn validate_queue(queue: &str) -> Result<()> {
    if queue.is_empty()
        || queue.len() > MAX_QUEUE_BYTES
        || queue.starts_with("amq.")
        || queue.ends_with(DEAD_SUFFIX)
        || !queue
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
    {
        return Err(BrokerError::InvalidQueue);
    }
    Ok(())
}

fn validate_payload(payload: &[u8]) -> Result<()> {
    if !(1..=MAX_PAYLOAD_BYTES).contains(&payload.len()) {
        return Err(BrokerError::InvalidPayload);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_authenticated_broker_urls_and_redacts_rejections() {
        for endpoint in [
            "amqp://worker:fixture@127.0.0.1:5672/%2f",
            "amqps://worker:encoded%40token@rabbit.example/vhost",
        ] {
            assert_eq!(validate_endpoint(endpoint), Ok(()));
        }
        for endpoint in [
            "amqp://localhost",
            "amqp://worker:@localhost",
            "amqp://:fixture@localhost",
            "https://worker:fixture@localhost",
            "amqp://worker:fixture@localhost/#fragment",
            "amqp://worker:fixture@local\nhost",
        ] {
            let error = validate_endpoint(endpoint).expect_err("invalid broker endpoint accepted");
            assert_eq!(error, BrokerError::InvalidEndpoint);
            assert!(!error.to_string().contains("fixture"));
        }
    }

    #[test]
    fn bounds_queue_and_payload_before_broker_access() {
        for queue in [
            "fleetworks.scope.jobs",
            "workspace_123-atlas",
            &"x".repeat(MAX_QUEUE_BYTES),
        ] {
            assert_eq!(validate_queue(queue), Ok(()));
        }
        for queue in [
            "",
            "amq.direct",
            "workspace.dead",
            "another workspace",
            &"x".repeat(MAX_QUEUE_BYTES + 1),
        ] {
            assert_eq!(validate_queue(queue), Err(BrokerError::InvalidQueue));
        }
        assert_eq!(validate_payload(&[0; MAX_PAYLOAD_BYTES]), Ok(()));
        assert_eq!(validate_payload(&[]), Err(BrokerError::InvalidPayload));
        assert_eq!(
            validate_payload(&[0; MAX_PAYLOAD_BYTES + 1]),
            Err(BrokerError::InvalidPayload)
        );
    }
}
