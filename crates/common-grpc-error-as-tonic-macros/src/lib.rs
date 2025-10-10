use tonic::metadata::MetadataMap;

pub static HEADER_SOURCE_DETAILS: &str = "x-md-source-details-bin";

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct SourceDetails {
    pub msg: String,
    pub source: String,
    pub location_file: String,
    pub location_line: u32,
    pub location_column: u32,
}

pub fn error_to_metadata<T>(error: T) -> MetadataMap
where
    T: std::error::Error,
{
    let mut metadata = MetadataMap::new();

    // store 'source' and `location' in metadata
    //   workaround of logging detailed error only on server side
    if let Some(source) = error.source() {
        let caller = std::panic::Location::caller();

        let debug = format!("{error:?}");

        let source_details = SourceDetails {
            msg: debug,
            source: source.to_string(),
            location_file: caller.file().to_string(),
            location_line: caller.line(),
            location_column: caller.column(),
        };

        let json = serde_json::to_string(&source_details);

        metadata.insert_bin(
            HEADER_SOURCE_DETAILS,
            ::tonic::metadata::MetadataValue::from_bytes(json.unwrap().as_bytes()),
        );
    }

    metadata
}
