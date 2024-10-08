use envconfig::Envconfig;

#[derive(Envconfig, Debug, Clone)]
pub struct FangExtConfig {
    #[envconfig(nested)]
    pub archiver: FangArchiverConfig,

    #[envconfig(nested)]
    pub cleaner: FangCleanerConfig,
}

#[derive(Envconfig, Debug, Clone)]
pub struct FangArchiverConfig {
    #[envconfig(from = "FANG_ARCHIVER_ENABLED", default = "true")]
    pub enabled: bool,

    #[envconfig(
        from = "FANG_ARCHIVER_SLEEP_SECONDS_ON_NOTHING_TO_MOVE",
        default = "3600"
    )]
    pub sleep_seconds_on_nothing_to_move: u16,

    #[envconfig(from = "FANG_ARCHIVER_SLEEP_SECONDS_ON_ERROR", default = "60")]
    pub sleep_seconds_on_error: u16,

    #[envconfig(from = "FANG_ARCHIVER_OLDER_THAN_HOURS", default = "1")]
    pub older_than_hours: u16,

    #[envconfig(from = "FANG_ARCHIVER_ROWS_TO_MOVE", default = "100")]
    pub rows_to_move: u16,
}

#[derive(Envconfig, Debug, Clone)]
pub struct FangCleanerConfig {
    #[envconfig(from = "FANG_CLEANER_ENABLED", default = "true")]
    pub enabled: bool,

    #[envconfig(
        from = "FANG_CLEANER_SLEEP_SECONDS_ON_NOTHING_TO_DELETE",
        default = "3600"
    )]
    pub sleep_seconds_on_nothing_to_delete: u16,

    #[envconfig(from = "FANG_CLEANER_SLEEP_SECONDS_ON_ERROR", default = "60")]
    pub sleep_seconds_on_error: u16,

    #[envconfig(from = "FANG_CLEANER_OLDER_THAN_HOURS", default = "168")]
    pub older_than_hours: u16,

    #[envconfig(from = "FANG_CLEANER_ROWS_TO_DELETE", default = "100")]
    pub rows_to_delete: u16,
}
