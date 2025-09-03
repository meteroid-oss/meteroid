use common_logging::logging;
use std::sync::OnceLock;

static LOG_INIT: OnceLock<()> = OnceLock::new();

pub fn logging() {
    LOG_INIT.get_or_init(|| {
        logging::init_regular_logging();
    });
}
