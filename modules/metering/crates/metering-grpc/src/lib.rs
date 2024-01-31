pub mod meteroid {
    pub mod metering {
        pub mod v1 {
            tonic::include_proto!("meteroid.metering.v1");
        }
    }
}

#[cfg(not(any(feature = "server", feature = "client")))]
compile_error!("Either `server` or `client` feature must be enabled.");
