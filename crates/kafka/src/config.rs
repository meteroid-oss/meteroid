use envconfig::Envconfig;
use rdkafka::ClientConfig;

#[derive(Envconfig, Clone)]
pub struct KafkaConnectionConfig {
    #[envconfig(from = "KAFKA_BOOTSTRAP_SERVERS", default = "localhost:9092")]
    pub bootstrap_servers: String,

    #[envconfig(from = "KAFKA_SECURITY_PROTOCOL")]
    pub security_protocol: Option<String>,

    #[envconfig(from = "KAFKA_SASL_MECHANISM")]
    pub sasl_mechanism: Option<String>,

    #[envconfig(from = "KAFKA_SASL_USERNAME")]
    pub sasl_username: Option<String>,

    #[envconfig(from = "KAFKA_SASL_PASSWORD")]
    pub sasl_password: Option<String>,
}

impl KafkaConnectionConfig {
    pub fn to_client_config(&self) -> ClientConfig {
        let mut client_config = ClientConfig::new();

        client_config.set("bootstrap.servers", &self.bootstrap_servers);

        if self
            .security_protocol
            .as_ref()
            .map_or(false, |s| !s.is_empty())
        {
            client_config.set(
                "security.protocol",
                self.security_protocol.as_ref().unwrap(),
            );
        }

        if self
            .sasl_mechanism
            .as_ref()
            .map_or(false, |s| !s.is_empty())
        {
            client_config.set("sasl.mechanism", self.sasl_mechanism.as_ref().unwrap());
        }

        if self.sasl_username.as_ref().map_or(false, |s| !s.is_empty()) {
            client_config.set("sasl.username", self.sasl_username.as_ref().unwrap());
        }

        if self.sasl_password.as_ref().map_or(false, |s| !s.is_empty()) {
            client_config.set("sasl.password", self.sasl_password.as_ref().unwrap());
        }

        client_config
    }
}
