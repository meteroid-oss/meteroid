use crate::config::Config;
use crate::error::OpenstackAdapterError;
use lapin::{Connection, ConnectionProperties};

pub struct RabbitSource {
    pub(crate) connection: Connection,
}

impl RabbitSource {
    pub async fn connect(config: &Config) -> Result<Self, OpenstackAdapterError> {
        let addr = &config.rabbit_addr;
        let conn = Connection::connect(addr, ConnectionProperties::default())
            .await
            .map_err(OpenstackAdapterError::LapinError)?;
        Ok(RabbitSource { connection: conn })
    }
}
