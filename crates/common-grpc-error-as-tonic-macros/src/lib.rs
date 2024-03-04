use tonic::metadata::MetadataMap;

pub static HEADER_SOURCE_DETAILS: &str = "x-md-source-details-bin";

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct SourceDetails {
    pub source: String,
    pub location: String,
}

pub fn error_to_metadata<T>(error: T) -> MetadataMap
where
    T: std::error::Error,
{
    let mut metadata = MetadataMap::new();

    // store 'source' and `location' in metadata
    //   workaround of logging detailed error only on server side
    if let Some(source) = error.source() {
        let source_details = SourceDetails {
            source: source.to_string(),
            location: ::std::panic::Location::caller().to_string(),
        };

        let json = serde_json::to_string(&source_details);

        metadata.insert_bin(
            HEADER_SOURCE_DETAILS,
            ::tonic::metadata::MetadataValue::from_bytes(json.unwrap().as_bytes()),
        );
    }

    metadata
}
