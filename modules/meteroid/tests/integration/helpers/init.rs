use common_logging::init::init_regular_logging;
use std::sync::OnceLock;

static LOG_INIT: OnceLock<()> = OnceLock::new();

pub fn logging() {
    LOG_INIT.get_or_init(|| {
        init_regular_logging();
    });
}
