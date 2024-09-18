#![allow(non_snake_case)]

macro_rules! include_proto_serde {
    ($package: tt) => {
        include!(concat!(env!("OUT_DIR"), concat!("/", $package, ".rs")));
        include!(concat!(
            env!("OUT_DIR"),
            concat!("/", $package, ".serde.rs")
        ));
    };
}

pub mod meteroid {
    pub mod api {
        pub mod addons {
            pub mod v1 {
                tonic::include_proto!("meteroid.api.addons.v1");
            }
        }

        pub mod apitokens {
            pub mod v1 {
                tonic::include_proto!("meteroid.api.apitokens.v1");
            }
        }

        pub mod adjustments {
            pub mod v1 {
                include_proto_serde!("meteroid.api.adjustments.v1");
            }
        }

        pub mod billablemetrics {
            pub mod v1 {
                include_proto_serde!("meteroid.api.billablemetrics.v1");
            }
        }

        pub mod customers {
            pub mod v1 {
                include_proto_serde!("meteroid.api.customers.v1");
            }
        }

        pub mod coupons {
            pub mod v1 {
                tonic::include_proto!("meteroid.api.coupons.v1");
            }
        }

        pub mod instance {
            pub mod v1 {
                tonic::include_proto!("meteroid.api.instance.v1");
            }
        }

        pub mod invoices {
            pub mod v1 {
                tonic::include_proto!("meteroid.api.invoices.v1");
            }
        }

        pub mod invoicingentities {
            pub mod v1 {
                tonic::include_proto!("meteroid.api.invoicingentities.v1");
            }
        }

        pub mod organizations {
            pub mod v1 {
                tonic::include_proto!("meteroid.api.organizations.v1");
            }
        }

        pub mod plans {
            pub mod v1 {
                tonic::include_proto!("meteroid.api.plans.v1");
            }
        }

        pub mod components {
            pub mod v1 {
                tonic::include_proto!("meteroid.api.components.v1");
            }
        }

        pub mod schedules {
            pub mod v1 {
                include_proto_serde!("meteroid.api.schedules.v1");
            }
        }

        pub mod productfamilies {
            pub mod v1 {
                tonic::include_proto!("meteroid.api.productfamilies.v1");
            }
        }

        pub mod products {
            pub mod v1 {
                tonic::include_proto!("meteroid.api.products.v1");
            }
        }

        pub mod subscriptions {
            pub mod v1 {
                tonic::include_proto!("meteroid.api.subscriptions.v1");
            }
        }

        pub mod stats {
            pub mod v1 {
                tonic::include_proto!("meteroid.api.stats.v1");
            }
        }

        pub mod tenants {
            pub mod v1 {
                include_proto_serde!("meteroid.api.tenants.v1");
            }
        }

        pub mod users {
            pub mod v1 {
                tonic::include_proto!("meteroid.api.users.v1");
            }
        }

        pub mod webhooks {
            pub mod out {
                pub mod v1 {
                    tonic::include_proto!("meteroid.api.webhooks.out.v1");
                }
            }
        }

        pub mod shared {
            pub mod v1 {
                include_proto_serde!("meteroid.api.shared.v1");

                impl BillingPeriod {
                    pub fn months_value(&self) -> u32 {
                        match self {
                            BillingPeriod::Monthly => 1,
                            BillingPeriod::Quarterly => 3,
                            BillingPeriod::Annual => 12,
                        }
                    }
                }
            }
        }
    }

    pub mod internal {
        pub mod v1 {
            tonic::include_proto!("meteroid.internal.v1");
        }
    }
}

pub mod _reflection {
    pub const FILE_DESCRIPTOR_SET: &[u8] =
        tonic::include_file_descriptor_set!("meteroid-grpc.protoset");
}
