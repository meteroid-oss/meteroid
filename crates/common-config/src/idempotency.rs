use envconfig::Envconfig;
use humantime::Duration;

#[derive(Envconfig, Debug, Clone)]
pub struct IdempotencyConfig {
    #[envconfig(from = "IDEMPOTENCY_REQUIRED", default = "false")]
    pub required: bool,

    #[envconfig(from = "IDEMPOTENCY_TTL", default = "1h")]
    pub ttl: Duration,

    #[envconfig(from = "IDEMPOTENCY_SIZE", default = "100000")]
    pub size: u64,
}

// workaround for idempotency cache function
static CONFIG: std::sync::OnceLock<IdempotencyConfig> = std::sync::OnceLock::new();

impl IdempotencyConfig {
    pub fn get() -> &'static Self {
        CONFIG.get_or_init(|| IdempotencyConfig::init_from_env().unwrap())
    }

    pub fn set(config: IdempotencyConfig) -> &'static Self {
        match CONFIG.get() {
            None => {
                CONFIG.set(config).expect("Failed to set config value");
                IdempotencyConfig::get()
            }
            Some(v) => {
                panic!("Config value is already set {:?}", v);
            }
        }
    }
}
