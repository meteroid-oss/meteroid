use log::{Level, MetadataBuilder, Record, logger};

// Extension to log and swallow error
pub trait UnwrapLogger<T, E>: Sized
where
    T: Default,
{
    fn unwrap_to_log<F: FnOnce(E) -> String>(self, level: Level, msg: F);

    #[inline(always)]
    #[track_caller]
    fn unwrap_to_log_error<F: FnOnce(E) -> String>(self, msg: F) {
        self.unwrap_to_log(Level::Error, msg)
    }

    #[inline(always)]
    #[track_caller]
    fn unwrap_to_log_warn<F: FnOnce(E) -> String>(self, msg: F) {
        self.unwrap_to_log(Level::Warn, msg)
    }

    #[inline(always)]
    #[track_caller]
    fn unwrap_to_default_warn<F: FnOnce(E) -> String>(self, log: F) -> T {
        self.unwrap_to_log(Level::Warn, log);
        T::default()
    }
}

impl<T, E> UnwrapLogger<T, E> for Result<T, E>
where
    T: Default,
{
    #[inline(always)]
    #[track_caller]
    fn unwrap_to_log<F: FnOnce(E) -> String>(self, level: Level, msg: F) {
        match self {
            Ok(_) => (),
            Err(err) => log_message(level, msg(err)),
        }
    }

    #[inline(always)]
    #[track_caller]
    fn unwrap_to_default_warn<F: FnOnce(E) -> String>(self, log: F) -> T {
        match self {
            Ok(t) => t,
            Err(err) => {
                log_message(Level::Warn, log(err));
                T::default()
            }
        }
    }
}

#[inline(always)]
#[track_caller]
fn log_message(level: Level, msg: String) {
    let loc = std::panic::Location::caller();

    logger().log(
        &Record::builder()
            .metadata(
                MetadataBuilder::new()
                    .target(loc.file())
                    .level(level)
                    .build(),
            )
            .args(format_args!("{}", msg))
            .file(Some(loc.file()))
            .line(Some(loc.line()))
            // .level(level)
            .build(),
    );
}
