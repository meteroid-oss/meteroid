use crate::config::Config;
use crate::sink::MeteroidSink;
use crate::source::RabbitSource;
use futures_lite::stream::StreamExt;
use lapin::{Channel, Consumer, options::*, types::FieldTable};
use serde::{Deserialize, Deserializer};
use std::collections::HashMap;
use std::fmt;
use tonic::Request;

use crate::error::OpenstackAdapterError;
use metering_grpc::meteroid::metering::v1 as server;
use metering_grpc::meteroid::metering::v1::IngestRequest;

pub struct EventHandler {
    pub sink: MeteroidSink,
    pub source: RabbitSource,
    pub config: Config,
}

impl EventHandler {
    pub async fn start(&mut self) -> Result<(), OpenstackAdapterError> {
        let conn = &self.source.connection;

        let event_channel = conn
            .create_channel()
            .await
            .map_err(OpenstackAdapterError::LapinError)?;
        let queue = &self.config.rabbit_queue.clone();

        self.consume_events(event_channel, queue).await?;

        Ok(())
    }

    async fn consume_events(
        &mut self,
        channel: Channel,
        queue: &str,
    ) -> Result<(), OpenstackAdapterError> {
        let consumer = channel
            .basic_consume(
                queue,
                "openstack_meteroid_consumer",
                BasicConsumeOptions::default(),
                FieldTable::default(),
            )
            .await
            .map_err(OpenstackAdapterError::LapinError)?;

        self.handle_messages(consumer).await?;

        Ok(())
    }

    // TODO error handling, DLQ

    async fn handle_messages(
        &mut self,
        mut consumer: Consumer,
    ) -> Result<(), OpenstackAdapterError> {
        while let Some(delivery) = consumer.next().await {
            let delivery = delivery.map_err(OpenstackAdapterError::LapinError)?;

            let oslo_event: OsloRecord = serde_json::from_slice(&delivery.data).map_err(|e| {
                OpenstackAdapterError::SerializationError(
                    "Failed to deserialize oslo event".to_string(),
                    e,
                )
            })?;

            let event: CeilometerOsloMessage =
                serde_json::from_str(&oslo_event.message).map_err(|e| {
                    OpenstackAdapterError::SerializationError(
                        "Failed to deserialize oslo message".to_string(),
                        e,
                    )
                })?;

            let events: Vec<server::Event> = match event.event_type {
                CeilometerEventType::Metering => {
                    let payloads: Vec<CeilometerMetricPayloadItem> = event
                        .payload
                        .iter()
                        .map(|x| serde_json::from_value(x.clone()).unwrap())
                        .collect();
                    payloads
                        .into_iter()
                        .map(|x| self.process_sample(x))
                        .collect::<Result<Vec<Option<server::Event>>, OpenstackAdapterError>>()?
                        .into_iter()
                        .flatten()
                        .collect()
                }
                CeilometerEventType::Event => {
                    let payloads: Vec<CeilometerEventPayloadItem> = event
                        .payload
                        .iter()
                        .map(|x| serde_json::from_value(x.clone()).unwrap())
                        .collect();
                    payloads
                        .into_iter()
                        .map(|x| self.process_event(x))
                        .collect::<Result<Vec<Option<server::Event>>, OpenstackAdapterError>>()?
                        .into_iter()
                        .flatten()
                        .collect()
                }
            };

            if !events.is_empty() {
                let res = self
                    .sink
                    .client
                    .ingest(Request::new(IngestRequest {
                        events,
                        allow_backfilling: false,
                    }))
                    .await
                    .map_err(OpenstackAdapterError::GrpcError)?;

                // TODO handle failures (DLQ in rabbit or on ingest service side)
                log::error!("Ingest response: {:?}", res.into_inner().failures);
            }

            delivery
                .ack(BasicAckOptions::default())
                .await
                .map_err(OpenstackAdapterError::LapinError)?;
        }

        Ok(())
    }

    fn process_sample(
        &mut self,
        sample: CeilometerMetricPayloadItem,
    ) -> Result<Option<server::Event>, OpenstackAdapterError> {
        let mut properties = HashMap::new();

        if sample.counter_volume == 0.0 {
            return Ok(None);
        }

        match sample.counter_name.as_str() {
            "network.outgoing.bytes.delta" => {
                properties.insert("unit".to_string(), sample.counter_unit.clone());
                properties.insert("resource_id".to_string(), sample.resource_id.clone());
                properties.insert("value".to_string(), sample.counter_volume.to_string());
            }
            _ => {
                log::info!("Unhandled counter name: {}", sample.counter_name);
                return Ok(None);
            }
        }

        Ok(Some(server::Event {
            id: sample.message_id.clone(),
            code: format!("openstack.{}", sample.counter_name),
            customer_id: Some(server::event::CustomerId::ExternalCustomerAlias(
                sample.project_id.clone(),
            )),
            timestamp: sample.timestamp.clone(),
            properties,
        }))
    }

    fn process_event(
        &mut self,
        event: CeilometerEventPayloadItem,
    ) -> Result<Option<server::Event>, OpenstackAdapterError> {
        // the project mapped to a customer external id. Later, we'll want to map this to a subscription extra field to allow multiple isolated projects per customer
        let timestamp = &event.generated;
        let mut properties = HashMap::new();
        match event.event_type.as_str() {
            "compute.instance.create.end"
            | "compute.instance.delete.end"
            | "compute.instance.resize.confirm.end" => {
                let project_id = event
                    .traits
                    .iter()
                    .find(|x| x.name == "project_id" || x.name == "tenant_id")
                    .and_then(|x| x.value.as_str())
                    .ok_or_else(|| {
                        OpenstackAdapterError::HandlerError(format!(
                            "Failed to decode Project ID - {:?}",
                            event.clone()
                        ))
                    })?;

                let instance_id = event
                    .traits
                    .iter()
                    .find(|x| x.name == "instance_id" || x.name == "resource_id")
                    .and_then(|x| x.value.as_str())
                    .ok_or_else(|| {
                        OpenstackAdapterError::HandlerError(
                            "Failed to decode Instance ID".to_string(),
                        )
                    })?;
                let flavor = event
                    .traits
                    .iter()
                    .find(|x| x.name == "instance_type")
                    .and_then(|x| x.value.as_str())
                    .ok_or_else(|| {
                        OpenstackAdapterError::HandlerError(
                            "Failed to decode Instance flavor".to_string(),
                        )
                    })?;

                properties.insert("instance_id".to_string(), instance_id.to_string());
                properties.insert("flavor".to_string(), flavor.to_string());

                Ok(Some(server::Event {
                    id: event.message_id.clone(),
                    code: format!("openstack.{}", event.event_type),
                    customer_id: Some(server::event::CustomerId::ExternalCustomerAlias(
                        project_id.to_string(),
                    )),
                    timestamp: timestamp.clone(),
                    properties,
                }))
            }

            //     "compute.instance.power_off.end" | "compute.instance.power_on.end"
            _ => {
                log::info!("Unhandled event type: {}", event.event_type);
                Ok(None)
            }
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct OsloRecord {
    // #[serde(rename = "oslo.version")]
    // version: String,
    #[serde(rename = "oslo.message")]
    message: String,
}

#[derive(Debug, Deserialize)]
pub struct CeilometerOsloMessage {
    pub _unique_id: String,
    pub event_type: CeilometerEventType, // "metering" , not super useful here (unless it's different
    // pub message_id: String,
    pub payload: Vec<serde_json::Value>, // CeilometerMetricPayloadItem or CeilometerEventPayloadItem based on event_type
                                         // pub priority: String,
                                         // pub publisher_id: String,
                                         // pub timestamp: String,
}

#[derive(Debug, Deserialize)]
pub enum CounterType {
    #[serde(rename = "gauge")]
    Gauge,
    #[serde(rename = "delta")]
    Delta,
    #[serde(rename = "cumulative")]
    Cumulative,
}

impl fmt::Display for CounterType {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            CounterType::Gauge => write!(f, "gauge"),
            CounterType::Delta => write!(f, "delta"),
            CounterType::Cumulative => write!(f, "cumulative"),
        }
    }
}

#[derive(Debug, Deserialize)]
pub enum CeilometerEventType {
    #[serde(rename = "metering")]
    Metering,
    #[serde(rename = "event")]
    Event,
}

#[derive(Debug, Deserialize)]
pub struct CeilometerMetricPayloadItem {
    pub counter_name: String,
    // pub counter_type: CounterType,
    pub counter_unit: String,
    pub counter_volume: f64,
    pub message_id: String,
    // pub message_signature: String,
    // pub monotonic_time: Option<String>,
    pub project_id: String,
    // pub project_name: Option<String>,
    pub resource_id: String,
    // pub resource_metadata: HashMap<String, serde_json::Value>, // string, int, datetime or an array of that
    // pub source: String,
    pub timestamp: String,
    // pub user_id: Option<String>,
    // pub user_name: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct CeilometerEventPayloadItem {
    pub message_id: String,
    pub event_type: String,
    pub generated: String, // UTC time for when the event occurred.
    pub traits: Vec<Trait>,
    // pub raw: serde_json::Value,
    // pub message_signature: String,
}

#[derive(Debug, Clone)]
struct Trait {
    name: String,
    value: serde_json::Value,
}

impl<'de> Deserialize<'de> for Trait {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Deserialize::deserialize(deserializer)
            .map(|TraitJson(name, _dtype, value)| Trait { name, value })
    }
}

#[derive(Debug, Deserialize)]
pub struct TraitJson(pub String, pub i32, pub serde_json::Value);
