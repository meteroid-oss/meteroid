use utoipa::ToSchema;

#[derive(ToSchema, serde::Serialize, serde::Deserialize)]
pub struct ProductFamilyListRequest {}

#[derive(Clone, ToSchema, serde::Serialize, serde::Deserialize)]
pub struct ProductFamily {
    pub local_id: String,
    pub name: String,
}
