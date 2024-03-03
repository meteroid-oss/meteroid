#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct SourceDetails {
    pub source: String,
    pub location: String,
}

pub trait SourceDetailsError {
    fn as_metadata_map(&self) -> tonic::metadata::MetadataMap;

    fn as_status(&self, code: tonic::Code) -> tonic::Status;
}
