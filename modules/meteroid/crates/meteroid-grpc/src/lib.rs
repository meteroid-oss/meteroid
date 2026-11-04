#![allow(non_snake_case)]
// prost-generated oneof variants can grow large as we add fields; boxing them at the
// .proto level is awkward and uniformly enlarging unrelated enums isn't worth it.
#![allow(clippy::large_enum_variant)]

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
        pub mod activation {
            pub mod v1 {
                tonic::include_proto!("meteroid.api.activation.v1");
            }
        }

        pub mod activity {
            pub mod v1 {
                tonic::include_proto!("meteroid.api.activity.v1");
            }
        }

        pub mod addons {
            pub mod v1 {
                tonic::include_proto!("meteroid.api.addons.v1");
            }
        }

        pub mod alerts {
            pub mod v1 {
                tonic::include_proto!("meteroid.api.alerts.v1");
            }
        }

        pub mod analytics {
            pub mod v1 {
                tonic::include_proto!("meteroid.api.analytics.v1");
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

        pub mod bankaccounts {
            pub mod v1 {
                tonic::include_proto!("meteroid.api.bankaccounts.v1");
            }
        }

        pub mod batchjobs {
            pub mod v1 {
                tonic::include_proto!("meteroid.api.batchjobs.v1");
            }
        }

        pub mod billablemetrics {
            pub mod v1 {
                include_proto_serde!("meteroid.api.billablemetrics.v1");
            }
        }

        pub mod connectors {
            pub mod v1 {
                tonic::include_proto!("meteroid.api.connectors.v1");
            }
        }

        pub mod customers {
            pub mod v1 {
                include_proto_serde!("meteroid.api.customers.v1");
            }
        }

        pub mod customeremails {
            pub mod v1 {
                tonic::include_proto!("meteroid.api.customeremails.v1");
            }
        }

        pub mod customproperties {
            pub mod v1 {
                tonic::include_proto!("meteroid.api.customproperties.v1");
            }
        }

        pub mod connect {
            pub mod v1 {
                tonic::include_proto!("meteroid.api.connect.v1");
            }
        }

        pub mod coupons {
            pub mod v1 {
                tonic::include_proto!("meteroid.api.coupons.v1");
            }
        }

        pub mod creditnotes {
            pub mod v1 {
                tonic::include_proto!("meteroid.api.creditnotes.v1");
            }
        }

        pub mod wallets {
            pub mod v1 {
                tonic::include_proto!("meteroid.api.wallets.v1");
            }
        }

        pub mod walletcontrols {
            pub mod v1 {
                tonic::include_proto!("meteroid.api.walletcontrols.v1");
            }
        }

        pub mod creditunits {
            pub mod v1 {
                tonic::include_proto!("meteroid.api.creditunits.v1");
            }
        }

        pub mod budgets {
            pub mod v1 {
                tonic::include_proto!("meteroid.api.budgets.v1");
            }
        }

        pub mod events {
            pub mod v1 {
                tonic::include_proto!("meteroid.api.events.v1");
            }
        }

        pub mod eventinsights {
            pub mod v1 {
                tonic::include_proto!("meteroid.api.eventinsights.v1");
            }
        }

        pub mod domains {
            pub mod v1 {
                tonic::include_proto!("meteroid.api.domains.v1");
            }
        }

        pub mod features {
            pub mod v1 {
                tonic::include_proto!("meteroid.api.features.v1");
            }
        }

        pub mod entitlements {
            pub mod v1 {
                tonic::include_proto!("meteroid.api.entitlements.v1");
            }
        }

        pub mod licenses {
            pub mod v1 {
                tonic::include_proto!("meteroid.api.licenses.v1");
            }
        }

        pub mod licensing {
            pub mod v1 {
                tonic::include_proto!("meteroid.api.licensing.v1");
            }
        }

        pub mod purchaselinks {
            pub mod v1 {
                tonic::include_proto!("meteroid.api.purchaselinks.v1");
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

        pub mod consolidation {
            pub mod v1 {
                tonic::include_proto!("meteroid.api.consolidation.v1");
            }
        }

        pub mod invoiceannextemplates {
            pub mod v1 {
                tonic::include_proto!("meteroid.api.invoiceannextemplates.v1");
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

        pub mod paymentreminders {
            pub mod v1 {
                tonic::include_proto!("meteroid.api.paymentreminders.v1");
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

        pub mod security {
            pub mod v1 {
                tonic::include_proto!("meteroid.api.security.v1");
            }
        }

        pub mod prices {
            pub mod v1 {
                tonic::include_proto!("meteroid.api.prices.v1");
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

        pub mod quotes {
            pub mod v1 {
                tonic::include_proto!("meteroid.api.quotes.v1");
            }
        }

        pub mod rbac {
            pub mod v1 {
                tonic::include_proto!("meteroid.api.rbac.v1");
            }
        }

        pub mod listviews {
            pub mod v1 {
                tonic::include_proto!("meteroid.api.listviews.v1");
            }
        }

        pub mod savedviews {
            pub mod v1 {
                tonic::include_proto!("meteroid.api.savedviews.v1");
            }
        }

        pub mod notifications {
            pub mod v1 {
                tonic::include_proto!("meteroid.api.notifications.v1");
            }
        }

        pub mod segments {
            pub mod v1 {
                tonic::include_proto!("meteroid.api.segments.v1");
            }
        }

        pub mod signals {
            pub mod v1 {
                tonic::include_proto!("meteroid.api.signals.v1");
            }
        }

        pub mod sso {
            pub mod v1 {
                tonic::include_proto!("meteroid.api.sso.v1");
            }
        }

        pub mod targets {
            pub mod v1 {
                tonic::include_proto!("meteroid.api.targets.v1");
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

        pub mod taxes {
            pub mod v1 {
                tonic::include_proto!("meteroid.api.taxes.v1");
            }
        }

        pub mod taxreports {
            pub mod v1 {
                tonic::include_proto!("meteroid.api.taxreports.v1");
            }
        }

        pub mod tenants {
            pub mod v1 {
                tonic::include_proto!("meteroid.api.tenants.v1");
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
                            BillingPeriod::Semiannual => 6,
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

    pub mod portal {
        pub mod checkout {
            pub mod v1 {
                tonic::include_proto!("meteroid.portal.checkout.v1");
            }
        }
        pub mod customer {
            pub mod v1 {
                tonic::include_proto!("meteroid.portal.customer.v1");
            }
        }
        pub mod purchase {
            pub mod v1 {
                tonic::include_proto!("meteroid.portal.purchase.v1");
            }
        }
        pub mod invoice {
            pub mod v1 {
                tonic::include_proto!("meteroid.portal.invoice.v1");
            }
        }
        pub mod shared {
            pub mod v1 {
                tonic::include_proto!("meteroid.portal.shared.v1");
            }
        }
        pub mod budgets {
            pub mod v1 {
                tonic::include_proto!("meteroid.portal.budgets.v1");
            }
        }
        pub mod wallets {
            pub mod v1 {
                tonic::include_proto!("meteroid.portal.wallets.v1");
            }
        }
        pub mod walletcontrols {
            pub mod v1 {
                tonic::include_proto!("meteroid.portal.walletcontrols.v1");
            }
        }
        pub mod quotes {
            pub mod v1 {
                tonic::include_proto!("meteroid.portal.quotes.v1");
            }
        }
        pub mod subscription {
            pub mod v1 {
                tonic::include_proto!("meteroid.portal.subscription.v1");
            }
        }
        pub mod dashboard {
            pub mod v1 {
                tonic::include_proto!("meteroid.portal.dashboard.v1");
            }
        }
    }
}

pub mod _reflection {
    pub const FILE_DESCRIPTOR_SET: &[u8] =
        tonic::include_file_descriptor_set!("meteroid-grpc.protoset");
}
