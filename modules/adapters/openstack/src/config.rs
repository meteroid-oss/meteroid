use envconfig::Envconfig;

#[derive(Envconfig, Debug, Clone)]
pub struct Config {
    #[envconfig(from = "METERING_API_EXTERNAL_URL")]
    pub metering_endpoint: String,

    #[envconfig(from = "METERING_API_KEY")]
    pub api_key: String,

    #[envconfig(from = "RABBIT_ADDR")]
    pub rabbit_addr: String,

    // TODO allow multiple queues ?
    #[envconfig(from = "RABBIT_QUEUE")]
    pub rabbit_queue: String,
}
