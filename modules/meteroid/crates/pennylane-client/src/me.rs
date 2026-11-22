use crate::client::PennylaneClient;
use crate::error::PennylaneError;
use secrecy::SecretString;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct Me {
    pub company: MeCompany,
    #[serde(default)]
    pub scopes: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MeCompany {
    pub id: i64,
    pub name: String,
}

impl PennylaneClient {
    /// <https://pennylane.readme.io/reference/getme>
    pub async fn get_me(&self, access_token: &SecretString) -> Result<Me, PennylaneError> {
        self.get("/api/external/v2/me", access_token, &()).await
    }
}
