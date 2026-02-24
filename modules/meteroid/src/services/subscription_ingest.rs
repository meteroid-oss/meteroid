use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use chrono::NaiveDate;
use common_domain::ids::{AliasOr, CustomerId, PlanId, PlanVersionId, TenantId};
use csv::ReaderBuilder;
use error_stack::bail;
use futures::StreamExt;
use meteroid_store::domain::enums::SubscriptionActivationCondition;
use meteroid_store::errors::StoreError;
use meteroid_store::repositories::{CustomersInterface, PlansInterface};
use meteroid_store::{Services, StoreResult};
use serde::Deserialize;
use uuid::Uuid;

use super::idempotency::IdempotencyService;

use meteroid_store::domain::{CreateSubscription, PaymentMethodsConfig};

use super::csv_ingest::{
    CsvString, optional_csv_string, optional_naive_date, optional_u16, optional_u32,
};

const MAX_CSV_SIZE: usize = 10 * 1024 * 1024; // 10MB limit
const CONCURRENCY: usize = 10;

pub struct SubscriptionIngestionOptions {
    pub delimiter: char,
    pub fail_on_error: bool,
}

pub struct SubscriptionIngestionFailure {
    pub row_number: i32,
    pub reason: String,
}

pub struct SubscriptionIngestionResult {
    pub total_rows: i32,
    pub successful_rows: i32,
    pub failures: Vec<SubscriptionIngestionFailure>,
}

struct ParsedRow {
    row_number: i32,
    csv: NewSubscriptionCsv,
}

struct CustomerValidatedRow {
    row_number: i32,
    csv: NewSubscriptionCsv,
    customer_id: CustomerId,
}

struct ValidatedRow {
    row_number: i32,
    csv: NewSubscriptionCsv,
    customer_id: CustomerId,
    plan_version_id: PlanVersionId,
}

#[derive(Clone)]
pub struct SubscriptionIngestService {
    services: Services,
    idempotency: Arc<dyn IdempotencyService>,
}

impl SubscriptionIngestService {
    pub fn new(services: Services, idempotency: Arc<dyn IdempotencyService>) -> Self {
        Self {
            services,
            idempotency,
        }
    }

    pub async fn ingest_csv(
        &self,
        tenant_id: TenantId,
        actor: Uuid,
        file_data: &[u8],
        options: SubscriptionIngestionOptions,
    ) -> StoreResult<SubscriptionIngestionResult> {
        let (raw_rows, mut failures) = Self::parse_csv(file_data, options.delimiter as u8)?;

        let total_rows = (raw_rows.len() + failures.len()) as i32;

        let proceed_on_failures =
            |f: &[SubscriptionIngestionFailure]| !options.fail_on_error || f.is_empty();

        let mut customer_validated_rows = vec![];
        if proceed_on_failures(&failures) {
            let (rows, errs) = self.validate_customers(tenant_id, raw_rows).await?;
            failures.extend(errs);
            customer_validated_rows = rows;
        }

        let mut validated_rows = vec![];
        if proceed_on_failures(&failures) {
            let (rows, errs) = self
                .validate_plan_versions(tenant_id, customer_validated_rows)
                .await?;
            failures.extend(errs);
            validated_rows = rows;
        }

        let parsed: Vec<(i32, Option<String>, CreateSubscription)> =
            if proceed_on_failures(&failures) {
                validated_rows
                    .into_iter()
                    .map(|row| {
                        let idempotency_key = row
                            .csv
                            .idempotency_key
                            .as_ref()
                            .map(|k| format!("{}:{}", tenant_id, k));
                        (
                            row.row_number,
                            idempotency_key,
                            Self::map_to_domain(
                                actor,
                                row.csv,
                                row.customer_id,
                                row.plan_version_id,
                            ),
                        )
                    })
                    .collect()
            } else {
                vec![]
            };

        tracing::info!("Processing {} subscription records", parsed.len());

        let mut successful_rows = 0;

        let mut stream = futures::stream::iter(parsed)
            .map(|(row_number, idempotency_key, sub)| {
                let services = self.services.clone();
                let idempotency = self.idempotency.clone();
                async move {
                    if let Some(key) = &idempotency_key
                        && idempotency
                            .check_and_set(key.clone(), Duration::from_secs(24 * 3600))
                            .await
                    {
                        return (row_number, idempotency_key, Ok(()));
                    }
                    let result = services
                        .insert_subscription(sub, tenant_id)
                        .await
                        .map(|_| ());
                    (row_number, idempotency_key, result)
                }
            })
            .buffer_unordered(CONCURRENCY);

        while let Some((row_number, idempotency_key, result)) = stream.next().await {
            match result {
                Ok(()) => successful_rows += 1,
                Err(e) => {
                    if let Some(key) = idempotency_key {
                        self.idempotency.invalidate(key).await;
                    }
                    failures.push(SubscriptionIngestionFailure {
                        row_number,
                        reason: e.to_string(),
                    });

                    if options.fail_on_error {
                        break;
                    }
                }
            }
        }

        Ok(SubscriptionIngestionResult {
            total_rows,
            successful_rows,
            failures,
        })
    }

    /// Resolves `customer_id_or_alias` for each row, returning validated rows with a resolved
    /// `CustomerId` and failures for any rows whose customer could not be found.
    async fn validate_customers(
        &self,
        tenant_id: TenantId,
        rows: Vec<ParsedRow>,
    ) -> StoreResult<(Vec<CustomerValidatedRow>, Vec<SubscriptionIngestionFailure>)> {
        let store = self.services.store();

        // Partition rows into those that already have an ID and those with an alias
        let mut id_rows: Vec<CustomerValidatedRow> = Vec::new();
        let mut alias_rows: Vec<(i32, NewSubscriptionCsv, String)> = Vec::new();

        for ParsedRow { row_number, csv } in rows {
            match csv.customer_id_or_alias.clone() {
                AliasOr::Id(id) => id_rows.push(CustomerValidatedRow {
                    row_number,
                    csv,
                    customer_id: id,
                }),
                AliasOr::Alias(alias) => alias_rows.push((row_number, csv, alias)),
            }
        }

        // Validate explicit IDs exist
        let id_set: Vec<CustomerId> = id_rows.iter().map(|r| r.customer_id).collect();
        let found_ids: std::collections::HashSet<CustomerId> = store
            .list_customers_by_ids(tenant_id, id_set)
            .await?
            .into_iter()
            .map(|c| c.id)
            .collect();

        // Resolve aliases to IDs
        let aliases: Vec<String> = alias_rows.iter().map(|(_, _, a)| a.clone()).collect();
        let alias_map: HashMap<String, CustomerId> = store
            .find_customer_ids_by_aliases(tenant_id, aliases)
            .await?
            .into_iter()
            .filter_map(|c| c.alias.map(|a| (a, c.id)))
            .collect();

        let mut valid = Vec::new();
        let mut failures = Vec::new();

        for row in id_rows {
            if found_ids.contains(&row.customer_id) {
                valid.push(row);
            } else {
                failures.push(SubscriptionIngestionFailure {
                    row_number: row.row_number,
                    reason: format!("Customer '{}' not found", row.customer_id),
                });
            }
        }

        for (row_number, csv, alias) in alias_rows {
            if let Some(customer_id) = alias_map.get(&alias).copied() {
                valid.push(CustomerValidatedRow {
                    row_number,
                    csv,
                    customer_id,
                });
            } else {
                failures.push(SubscriptionIngestionFailure {
                    row_number,
                    reason: format!("Customer with alias '{}' not found", alias),
                });
            }
        }

        Ok((valid, failures))
    }

    /// Resolves `plan_version_id` for each row using a single batched DB call.
    /// For rows with `plan_version: Some(v)`, finds the exact published version.
    /// For rows with `plan_version: None`, picks the latest published version of the plan.
    async fn validate_plan_versions(
        &self,
        tenant_id: TenantId,
        rows: Vec<CustomerValidatedRow>,
    ) -> StoreResult<(Vec<ValidatedRow>, Vec<SubscriptionIngestionFailure>)> {
        let store = self.services.store();

        let plan_ids: Vec<PlanId> = rows
            .iter()
            .map(|r| r.csv.plan_id)
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();

        // Fetch all published versions for the referenced plans in one query.
        // Build a map: plan_id -> BTreeMap<version, plan_version_id>
        let mut version_map: HashMap<PlanId, std::collections::BTreeMap<i32, PlanVersionId>> =
            HashMap::new();
        for row in store
            .list_published_versions_by_plan_ids(plan_ids, tenant_id)
            .await?
        {
            version_map
                .entry(row.plan_id)
                .or_default()
                .insert(row.version, row.id);
        }

        let mut valid = Vec::new();
        let mut failures = Vec::new();

        for CustomerValidatedRow {
            row_number,
            csv,
            customer_id,
        } in rows
        {
            let versions = version_map.get(&csv.plan_id);
            let resolved = match csv.plan_version {
                Some(v) => versions.and_then(|m| m.get(&(v as i32))).copied(),
                None => versions.and_then(|m| m.values().next_back()).copied(),
            };

            match resolved {
                Some(plan_version_id) => valid.push(ValidatedRow {
                    row_number,
                    plan_version_id,
                    csv,
                    customer_id,
                }),
                None => {
                    let reason = match csv.plan_version {
                        Some(v) => format!(
                            "No published version {} found for plan '{}'",
                            v, csv.plan_id
                        ),
                        None => format!("No published version found for plan '{}'", csv.plan_id),
                    };
                    failures.push(SubscriptionIngestionFailure { row_number, reason });
                }
            }
        }

        Ok((valid, failures))
    }

    fn parse_csv(
        file_data: &[u8],
        delimiter: u8,
    ) -> StoreResult<(Vec<ParsedRow>, Vec<SubscriptionIngestionFailure>)> {
        if file_data.is_empty() {
            bail!(StoreError::InvalidArgument("File is empty".to_string()));
        }

        if file_data.len() > MAX_CSV_SIZE {
            bail!(StoreError::InvalidArgument(format!(
                "File size exceeds maximum allowed ({MAX_CSV_SIZE} bytes)"
            )));
        }

        let mut reader = ReaderBuilder::new()
            .has_headers(true)
            .delimiter(delimiter)
            .from_reader(file_data);

        let mut parsed = Vec::new();
        let mut failures = Vec::new();
        let mut row_number = 2i32; // Account for header row (always present)

        for rec in reader.deserialize::<NewSubscriptionCsv>() {
            match rec {
                Ok(csv) => parsed.push(ParsedRow { row_number, csv }),
                Err(e) => failures.push(SubscriptionIngestionFailure {
                    row_number,
                    reason: format!("Failed to parse row: {e}"),
                }),
            }
            row_number += 1;
        }

        Ok((parsed, failures))
    }

    fn map_to_domain(
        actor: Uuid,
        csv: NewSubscriptionCsv,
        customer_id: CustomerId,
        plan_version_id: PlanVersionId,
    ) -> CreateSubscription {
        use meteroid_store::domain::SubscriptionNew;

        CreateSubscription {
            subscription: SubscriptionNew {
                customer_id,
                plan_version_id,
                created_by: actor,
                net_terms: csv.net_terms,
                invoice_memo: None,
                invoice_threshold: None,
                start_date: csv.start_date,
                end_date: csv.end_date,
                billing_start_date: None,
                activation_condition: csv.activation_condition.into(),
                trial_duration: None,
                billing_day_anchor: csv.billing_day_anchor,
                payment_methods_config: csv.payment_method.map(Into::into),
                auto_advance_invoices: csv.auto_advance_invoices,
                charge_automatically: csv.charge_automatically,
                purchase_order: csv.purchase_order.map(|s| s.0),
                backdate_invoices: false,
                skip_checkout_session: false,
                skip_past_invoices: csv.skip_past_invoices,
            },
            price_components: None,
            add_ons: None,
            coupons: None,
        }
    }
}

#[derive(Deserialize)]
pub struct NewSubscriptionCsv {
    #[serde(default, with = "optional_csv_string")]
    pub idempotency_key: Option<CsvString>,
    pub customer_id_or_alias: AliasOr<CustomerId>,
    pub plan_id: PlanId,
    #[serde(default, with = "optional_u32")]
    pub plan_version: Option<u32>,
    pub start_date: NaiveDate,
    pub activation_condition: ActivationConditionCsv,
    pub auto_advance_invoices: bool,
    #[serde(default, with = "optional_u16")]
    pub billing_day_anchor: Option<u16>,
    pub charge_automatically: bool,
    #[serde(default, with = "optional_naive_date")]
    pub end_date: Option<NaiveDate>,
    #[serde(default, with = "optional_u32")]
    pub net_terms: Option<u32>,
    #[serde(default)]
    pub payment_method: Option<PaymentMethodCsv>,
    #[serde(default, with = "optional_csv_string")]
    pub purchase_order: Option<CsvString>,
    pub skip_past_invoices: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ActivationConditionCsv {
    OnStartDate,
    OnCheckout,
    Manual,
}

impl From<ActivationConditionCsv> for SubscriptionActivationCondition {
    fn from(v: ActivationConditionCsv) -> Self {
        match v {
            ActivationConditionCsv::OnStartDate => SubscriptionActivationCondition::OnStart,
            ActivationConditionCsv::OnCheckout => SubscriptionActivationCondition::OnCheckout,
            ActivationConditionCsv::Manual => SubscriptionActivationCondition::Manual,
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PaymentMethodCsv {
    Online,
    BankTransfer,
    External,
}

impl From<PaymentMethodCsv> for PaymentMethodsConfig {
    fn from(v: PaymentMethodCsv) -> Self {
        match v {
            PaymentMethodCsv::Online => PaymentMethodsConfig::Online { config: None },
            PaymentMethodCsv::BankTransfer => {
                PaymentMethodsConfig::BankTransfer { account_id: None }
            }
            PaymentMethodCsv::External => PaymentMethodsConfig::External,
        }
    }
}
