use envconfig::Envconfig;
use rdkafka::ClientConfig;

#[derive(Envconfig, Clone, Debug)]
pub struct KafkaConnectionConfig {
    #[envconfig(from = "KAFKA_BOOTSTRAP_SERVERS")]
    pub bootstrap_servers: Option<String>,

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
    pub fn none() -> Self {
        KafkaConnectionConfig {
            bootstrap_servers: None,
            security_protocol: None,
            sasl_mechanism: None,
            sasl_username: None,
            sasl_password: None,
        }
    }

    pub fn is_none(&self) -> bool {
        self.bootstrap_servers.is_none()
    }

    pub fn to_client_config(&self) -> ClientConfig {
        let mut client_config = ClientConfig::new();

        let bootstrap_servers = self
            .bootstrap_servers
            .as_ref()
            .expect("Missing KAFKA_BOOTSTRAP_SERVERS env var");

        client_config.set("bootstrap.servers", bootstrap_servers);

        if self
            .security_protocol
            .as_ref()
            .is_some_and(|s| !s.is_empty())
        {
            client_config.set(
                "security.protocol",
                self.security_protocol.as_ref().unwrap(),
            );
        }

        if self.sasl_mechanism.as_ref().is_some_and(|s| !s.is_empty()) {
            client_config.set("sasl.mechanism", self.sasl_mechanism.as_ref().unwrap());
        }

        if self.sasl_username.as_ref().is_some_and(|s| !s.is_empty()) {
            client_config.set("sasl.username", self.sasl_username.as_ref().unwrap());
        }

        if self.sasl_password.as_ref().is_some_and(|s| !s.is_empty()) {
            client_config.set("sasl.password", self.sasl_password.as_ref().unwrap());
        }

        client_config
    }
}
