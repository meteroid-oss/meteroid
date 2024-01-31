use chrono::Utc;

pub fn timestamp_to_datetime(dt: prost_types::Timestamp) -> chrono::DateTime<Utc> {
    chrono::DateTime::<Utc>::from_timestamp(dt.seconds, dt.nanos as u32).unwrap()
}

pub fn datetime_to_timestamp(dt: chrono::DateTime<Utc>) -> prost_types::Timestamp {
    prost_types::Timestamp {
        seconds: dt.timestamp(),
        nanos: dt.timestamp_subsec_nanos() as i32,
    }
}
