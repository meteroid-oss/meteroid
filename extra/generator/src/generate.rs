use crate::domain::{Config, DataType, FixedValue, Property, Schema};
use std::str::FromStr;

use log::{error, info};
use tokio::task::JoinSet;
use uuid::Uuid;

use common_grpc::middleware::common::auth::API_KEY_HEADER;
use metering_grpc::meteroid::metering::v1::event::CustomerId;
use metering_grpc::meteroid::metering::v1::events_service_client::EventsServiceClient;
use metering_grpc::meteroid::metering::v1::{Event, IngestRequest};
use tokio::time::{Duration, sleep};
use tonic::metadata::MetadataValue;
use tonic::transport::Channel;

pub async fn generate_events(config: &Config) {
    let channel = Channel::from_shared(config.connect.endpoint.clone())
        .expect("Invalid ingest endpoint")
        .connect_lazy();

    let client = EventsServiceClient::new(channel);

    // arbitrary, supports more with a single process
    assert!(
        config.events_per_second <= 5000,
        "Can't generate more than 5000 events per second."
    );

    let max_batch_size = 500;

    let (batch_size, delay_per_batch) = if config.events_per_second < max_batch_size {
        (config.events_per_second, Duration::from_secs(1))
    } else {
        (
            max_batch_size,
            Duration::from_millis(u64::from(
                1000 / (config.events_per_second / max_batch_size),
            )),
        )
    };

    let start = std::time::Instant::now();
    let mut sent_events = 0;

    let api_key_metadata = MetadataValue::from_str(&config.connect.api_key).unwrap();

    let mut set = JoinSet::new();

    loop {
        let mut events = Vec::new();

        let mut iteration_batch_size = batch_size;

        if let Some(limit) = config.limit {
            if sent_events >= limit {
                break;
            }
            if (sent_events + batch_size) >= limit {
                iteration_batch_size = limit - sent_events;
            }
        }

        for _ in 0..iteration_batch_size {
            events.push(generate_any(&config.events));
            sent_events += 1;
        }

        let mut request = tonic::Request::new(IngestRequest {
            events,
            allow_backfilling: false,
        });
        let headers = request.metadata_mut();
        headers.insert(API_KEY_HEADER, api_key_metadata.clone());

        // Send batch to gRPC service

        let mut client_clone = client.clone();
        // spawn and forget

        let count = sent_events;
        set.spawn(async move {
            let ts = std::time::Instant::now();
            let response = client_clone.ingest(request).await;
            match response {
                Ok(_) => {
                    let now = std::time::Instant::now();
                    let diff = now - ts;
                    info!(
                        "Batch ingested in {}ms. {} events ingested in total.",
                        diff.as_millis(),
                        count
                    );
                }
                Err(e) => {
                    error!("Failed to ingest: {e:?}");
                }
            }
        });

        // Wait to maintain the rate of events per second
        sleep(delay_per_batch).await;
    }

    while let Some(res) = set.join_next().await {
        match res {
            Ok(()) => {}
            Err(err) => {
                error!("Join error while waiting on ACK: {err:?}");
            }
        }
    }

    let end = std::time::Instant::now();

    let diff = end - start;

    info!(
        "Completed ! {} events in {}ms",
        sent_events,
        diff.as_millis()
    );
}

fn generate_any(schemas: &Vec<Schema>) -> Event {
    let total_weight: f64 = schemas.iter().map(|s| s.weight.unwrap_or(1.0)).sum();
    let mut random = fastrand::f64() * total_weight;

    for schema in schemas {
        random -= schema.weight.unwrap_or(1.0);
        if random <= 0.0 {
            return generate_random_data(schema);
        }
    }

    panic!("No event schema provided");
}

fn generate_random_data(schema: &Schema) -> Event {
    let now = chrono::Utc::now();

    let mut properties = std::collections::HashMap::new();

    for (key, property) in &schema.properties {
        let value = match property {
            Property::Typed(data_type) => match data_type {
                DataType::int { min, max } => {
                    fastrand::i32(min.unwrap_or(0)..=max.unwrap_or(100)).to_string()
                }
                DataType::float { .. } => fastrand::f64().to_string(), // TODO
                DataType::bool => fastrand::bool().to_string(),
                DataType::string { length } => (0..length.unwrap_or(10))
                    .map(|_| fastrand::alphanumeric())
                    .collect(),
                DataType::pick { values } => match fastrand::choice(values) {
                    Some(FixedValue::Boolean(b)) => b.to_string(),
                    Some(FixedValue::String(s)) => s.clone(),
                    Some(FixedValue::Float(f)) => f.to_string(),
                    Some(FixedValue::Integer(i)) => i.to_string(),
                    None => String::new(),
                },
            },
            Property::Fixed(fixed_value) => match fixed_value {
                FixedValue::Boolean(b) => b.to_string(),
                FixedValue::String(s) => s.clone(),
                FixedValue::Float(f) => f.to_string(),
                FixedValue::Integer(i) => i.to_string(),
            },
        };

        properties.insert(key.clone(), value);
    }

    Event {
        id: Uuid::new_v4().to_string(),
        code: schema.code.clone(),
        customer_id: Some(CustomerId::ExternalCustomerAlias(
            schema.customer_aliases[fastrand::usize(0..schema.customer_aliases.len())].clone(),
        )),
        timestamp: now.to_rfc3339(),
        properties,
    }
}
