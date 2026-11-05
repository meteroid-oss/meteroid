use crate::pgmq::processors;
use hubspot_client::client::HubspotClient;
use meteroid_app::config::Config;
use meteroid_app::services::credit_note_rendering::CreditNotePdfRenderingService;
use meteroid_app::services::currency_rates::CurrencyRatesService;
use meteroid_app::services::egress::Egress;
use meteroid_app::services::idempotency::IdempotencyService;
use meteroid_app::services::instance_telemetry::InstanceTelemetryService;
use meteroid_app::services::invoice_rendering::PdfRenderingService;
use meteroid_app::services::storage::S3Storage;
use meteroid_kit::{Modules, WorkerContext};
use meteroid_mailer::service::MailerService;
use meteroid_store::Services;
use meteroid_store::domain::enums::BatchJobTypeEnum;
use meteroid_store::domain::pgmq::{PgmqQueue, QueueConsumer};
use pennylane_client::client::PennylaneClient;
use std::sync::Arc;
use strum::IntoEnumIterator;

pub mod batch_jobs;
pub mod billing;

pub mod misc;
pub mod pgmq;
pub mod run;
pub use meteroid_app::workers::utils;
pub mod webhook_delivery;

//
// #[derive(Debug, Clone, Envconfig)]
// struct WorkerConfig {
//     #[envconfig(from = "ENABLE_OUTBOX_WORKER", default = "true")]
//     enable_outbox_worker: bool,
//
//     #[envconfig(from = "ENABLE_PDF_WORKER", default = "true")]
//     enable_pdf_worker: bool,
//
//     #[envconfig(from = "ENABLE_WEBHOOK_WORKER", default = "true")]
//     enable_webhook_worker: bool,
//
//     #[envconfig(from = "ENABLE_LIFECYCLE_WORKER", default = "true")]
//     enable_lifecycle_worker: bool,
//
//     #[envconfig(from = "ENABLE_SCHEDULED_WORKER", default = "true")]
//     enable_scheduled_worker: bool,
//
//     #[envconfig(from = "WORKER_CONCURRENCY", default = "1")]
//     worker_concurrency: usize,
// }

#[allow(clippy::too_many_arguments)]
pub async fn spawn_workers(
    store: Arc<meteroid_store::Store>,
    services: Arc<Services>,
    metering: Arc<meteroid_app::clients::usage::MeteringUsageClient>,
    object_store_service: Arc<S3Storage>,
    currency_rates_service: Arc<dyn CurrencyRatesService>,
    pdf_rendering_service: Arc<PdfRenderingService>,
    credit_note_pdf_rendering_service: Arc<CreditNotePdfRenderingService>,
    mailer_service: Arc<dyn MailerService>,
    idempotency: Arc<dyn IdempotencyService>,
    config: &Config,
    modules: &Modules,
    egress: &Egress,
) -> Result<(), egress_http::EgressError> {
    let hubspot_client = Arc::new(HubspotClient::default());
    let pennylane_client = Arc::new(PennylaneClient::default());

    // TODO add config to only spawn some
    let mut join_set = tokio::task::JoinSet::new();
    let mut consumed: Vec<PgmqQueue> = Vec::new();

    let public_url = config.public_url.clone();
    let rest_api_external_url = config.rest_api_external_url.clone();
    let entitlements = modules.entitlements(&store);
    let analytics = modules.analytics_hook(&store, config, egress);

    // Outbox fan-out is done at publish time via pgmq topic routing
    // (producers call send_batch_topic in-tx), so there is no dispatcher worker.

    {
        let store = store.clone();
        consumed.push(PgmqQueue::InvoicePdfRequest);
        join_set.spawn(async move {
            processors::run_pdf_render(store, pdf_rendering_service).await;
        });
    }
    {
        let store = store.clone();
        consumed.push(PgmqQueue::CreditNotePdfRequest);
        join_set.spawn(async move {
            processors::run_credit_note_pdf_render(store, credit_note_pdf_rendering_service).await;
        });
    }
    {
        let delivery_worker = webhook_delivery::WebhookDeliveryWorker::new(store.clone(), egress)?;

        {
            let store = store.clone();
            consumed.push(PgmqQueue::WebhookOut);
            join_set.spawn(async move {
                processors::run_webhook_out(store).await;
            });
        }

        join_set.spawn(async move {
            delivery_worker.run().await;
        });
    }
    {
        if config.oauth.hubspot.is_enabled() {
            let store = store.clone();
            consumed.push(PgmqQueue::HubspotSync);
            join_set.spawn(async move {
                processors::run_hubspot_sync(store, hubspot_client).await;
            });
        } else {
            log::warn!("Hubspot OAuth is not configured: its sync queue is drained.");
        }
    }
    {
        if config.oauth.pennylane.is_enabled() {
            let store = store.clone();
            let object_store_service = object_store_service.clone();
            consumed.push(PgmqQueue::PennylaneSync);
            join_set.spawn(async move {
                processors::run_pennylane_sync(store, pennylane_client, object_store_service).await;
            });
        } else {
            log::warn!("Pennylane OAuth is not configured: its sync queue is drained.");
        }
    }
    {
        let store = store.clone();
        let services = services.clone();
        consumed.push(PgmqQueue::InvoiceOrchestration);
        join_set.spawn(async move {
            processors::run_invoice_orchestration(store, services).await;
        });
    }
    {
        let store = store.clone();
        consumed.push(PgmqQueue::VatValidation);
        join_set.spawn(async move {
            processors::run_vat_validation(store).await;
        });
    }
    {
        use meteroid_store::leader_lock::LeaderLock;
        use meteroid_store::leader_lock::PgLeaderElection;

        let store = store.clone();
        let usage_client = services.usage_clients();
        let analytics = analytics.clone();
        let object_store_service = object_store_service.clone();
        let elector = Arc::new(PgLeaderElection::new(
            store.pool.clone(),
            LeaderLock::TenantDeletionCleanup,
        ));
        let schedule = config.workers.tenant_deletion_cleanup_cron.clone();
        join_set.spawn(async move {
            misc::tenant_deletion_cleanup::run_tenant_deletion_cleanup_worker(
                store,
                usage_client,
                analytics,
                object_store_service,
                elector,
                schedule,
            )
            .await;
        });
    }
    {
        let store = store.clone();
        let services = services.clone();
        consumed.push(PgmqQueue::PaymentRequest);
        join_set.spawn(async move {
            processors::run_payment_request(store, services).await;
        });
    }
    {
        let store = store.clone();
        let services = services.clone();
        let object_store_service = object_store_service.clone();
        consumed.push(PgmqQueue::WebhookIn);
        join_set.spawn(async move {
            processors::run_webhook_in(store, services, object_store_service).await;
        });
    }
    {
        use meteroid_app::features::EntitlementsWhitelabelResolver;

        let store = store.clone();
        let services = services.clone();
        let object_store_service = object_store_service.clone();
        let whitelabel = Arc::new(EntitlementsWhitelabelResolver::new(
            store.clone(),
            entitlements.clone(),
        ));
        consumed.push(PgmqQueue::SendEmailRequest);
        join_set.spawn(async move {
            processors::run_email_sender(
                store,
                services,
                mailer_service,
                object_store_service,
                public_url,
                rest_api_external_url,
                whitelabel,
            )
            .await;
        });
    }
    {
        let store = store.clone();
        let services = services.clone();
        consumed.push(PgmqQueue::QuoteConversion);
        join_set.spawn(async move {
            processors::run_quote_conversion(store, services).await;
        });
    }

    {
        let store = store.clone();
        join_set.spawn(async move {
            misc::checkout_session_cleanup::run_checkout_session_cleanup_worker(store).await;
        });
    }

    {
        let store = store.clone();
        join_set.spawn(async move {
            misc::portal_token_cleanup::run_portal_token_cleanup_worker(store).await;
        });
    }

    {
        let store = store.clone();
        let services = services.clone();
        join_set.spawn(async move {
            misc::customer_credit::run_customer_credit_worker(store, services).await;
        });
    }

    {
        let services = services.clone();
        join_set.spawn(async move {
            misc::customer_credit::run_wallet_purchase_worker(services).await;
        });
    }

    {
        let services = services.clone();
        join_set.spawn(async move {
            misc::customer_credit::run_plan_change_settlement_worker(services).await;
        });
    }

    {
        use meteroid_store::leader_lock::LeaderLock;
        use meteroid_store::leader_lock::PgLeaderElection;

        let store = store.clone();
        let elector = Arc::new(PgLeaderElection::new(
            store.pool.clone(),
            LeaderLock::CreditInputFeed,
        ));
        join_set.spawn(async move {
            misc::credit_input_feed::run_credit_input_feed_worker(store, elector).await;
        });
    }

    {
        use meteroid_store::leader_lock::LeaderLock;
        use meteroid_store::leader_lock::PgLeaderElection;

        let store = store.clone();
        let elector = Arc::new(PgLeaderElection::new(
            store.pool.clone(),
            LeaderLock::AbandonedPurchaseCleanup,
        ));
        join_set.spawn(async move {
            misc::abandoned_purchase_cleanup::run_abandoned_purchase_cleanup_worker(store, elector)
                .await;
        });
    }

    {
        use meteroid_store::leader_lock::LeaderLock;
        use meteroid_store::leader_lock::PgLeaderElection;

        let store = store.clone();
        let elector = Arc::new(PgLeaderElection::new(
            store.pool.clone(),
            LeaderLock::VatRevalidation,
        ));
        let schedule = config.workers.vat_revalidation_cron.clone();
        join_set.spawn(async move {
            misc::vat_revalidation_worker::run_vat_revalidation_worker(store, elector, schedule)
                .await;
        });
    }

    if config.workers.draft_refresh_enabled {
        use meteroid_store::leader_lock::LeaderLock;
        use meteroid_store::leader_lock::PgLeaderElection;

        let services = services.clone();
        let elector = Arc::new(PgLeaderElection::new(
            store.pool.clone(),
            LeaderLock::DraftRefresh,
        ));
        let schedule = config.workers.draft_refresh_cron.clone();
        join_set.spawn(async move {
            misc::draft_refresh_worker::run_draft_refresh_worker(services, elector, schedule).await;
        });
    }

    {
        let ctx = WorkerContext {
            config: config.clone(),
            store: store.clone(),
            services: services.clone(),
            feature_resolver: entitlements.clone(),
            analytics: analytics.clone(),
            pending_usage: metering,
            egress: egress.clone(),
        };
        for module in &modules.workers {
            module.spawn(&ctx, &mut join_set);
        }
    }
    for queue in queues_to_drain(&consumed, |q| modules.consumes(q)) {
        let store = store.clone();
        join_set.spawn(async move {
            processors::run_noop(store, queue).await;
        });
    }

    // Batch job worker
    {
        let store = store.clone();
        let object_store_service = object_store_service.clone();
        let services = services.clone();
        let usage_client = services.usage_clients();
        let idempotency = idempotency.clone();
        let analytics = analytics.clone();
        join_set.spawn(async move {
            // Captured before the worker takes ownership of them below; the tax-report
            // export processor is the only one that needs its own object-store handle.
            let object_store_for_tax = object_store_service.clone();
            let store_for_tax = store.clone();

            let mut worker =
                batch_jobs::worker::BatchJobWorker::new(store.clone(), object_store_service);

            worker.register_processor(
                BatchJobTypeEnum::EventCsvImport,
                Arc::new(batch_jobs::processors::EventCsvProcessor::new(
                    usage_client,
                    store.clone(),
                )),
            );
            worker.register_processor(
                BatchJobTypeEnum::CustomerCsvImport,
                Arc::new(batch_jobs::processors::CustomerCsvProcessor::new(store)),
            );
            worker.register_processor(
                BatchJobTypeEnum::SubscriptionCsvImport,
                Arc::new(batch_jobs::processors::SubscriptionCsvProcessor::new(
                    (*services).clone(),
                    idempotency,
                    analytics,
                )),
            );
            worker.register_processor(
                BatchJobTypeEnum::TaxReportExport,
                Arc::new(batch_jobs::processors::TaxReportExportProcessor::new(
                    store_for_tax,
                    object_store_for_tax,
                )),
            );

            let worker = Arc::new(worker);
            worker.run().await;
        });
    }

    {
        let store = store.clone();
        join_set.spawn(async move {
            misc::currency_rates_worker::run_currency_rates_worker(&store, &currency_rates_service)
                .await;
        });
    }

    {
        use meteroid_store::leader_lock::{LeaderLock, PgLeaderElection};

        let telemetry = Arc::new(InstanceTelemetryService::new(
            (*store).clone(),
            config.instance_telemetry.clone(),
            config.object_store_uri.clone(),
            modules.ids(),
        ));
        let elector = Arc::new(PgLeaderElection::new(
            store.pool.clone(),
            LeaderLock::TelemetryReport,
        ));
        join_set.spawn(async move {
            misc::instance_telemetry_worker::run(telemetry, elector).await;
        });
    }

    {
        use meteroid_store::leader_lock::LeaderLock;
        use meteroid_store::leader_lock::PgLeaderElection;

        let store = store.clone();
        let services = services.clone();
        // Lost-return backstop for hosted in-flow captures on webhook-less
        // (`PollingRequired`) providers. Single replica, same kill switch as
        // reconciliation.
        let elector = Arc::new(PgLeaderElection::new(
            store.pool.clone(),
            LeaderLock::HostedPaymentSweep,
        ));
        let enabled = config.workers.reconciliation_enabled;
        join_set.spawn(async move {
            misc::hosted_payment_sweeper::run_hosted_payment_sweeper(services, elector, enabled)
                .await;
        });
    }

    {
        use meteroid_store::repositories::connectors::ConnectorsInterface;

        use distributed_lock::LeaderElection;
        use meteroid_store::leader_lock::LeaderLock;
        use meteroid_store::leader_lock::PgLeaderElection;

        let store = store.clone();
        // Endpoints registered by an older release only know that release's event types. One
        // replica is enough: the others would only replay the same idempotent provider calls.
        let elector =
            PgLeaderElection::new(store.pool.clone(), LeaderLock::WebhookSubscriptionSync);
        join_set.spawn(async move {
            let guard = match elector.try_acquire().await {
                Ok(Some(guard)) => guard,
                Ok(None) => return,
                Err(e) => {
                    log::warn!("Webhook subscription sync skipped, leader election failed: {e:?}");
                    return;
                }
            };
            match store.sync_all_webhook_subscriptions().await {
                Ok(n) => {
                    log::info!("Synced webhook subscriptions on {n} self-registered endpoints")
                }
                Err(e) => log::warn!("Webhook subscription sync failed: {e:?}"),
            }
            guard.release().await;
        });
    }
    {
        let services = services.clone();
        join_set.spawn(async move {
            billing::lifecycle::run_worker(services).await;
        });
    }
    {
        let services = services.clone();
        join_set.spawn(async move {
            billing::scheduled::run_worker(services).await;
        });
    }
    {
        use meteroid_store::leader_lock::LeaderLock;
        use meteroid_store::leader_lock::PgLeaderElection;

        let store = store.clone();
        let services = services.clone();
        let elector = Arc::new(PgLeaderElection::new(
            store.pool.clone(),
            LeaderLock::Reconciliation,
        ));
        let enabled = config.workers.reconciliation_enabled;
        join_set.spawn(async move {
            misc::reconciliation_worker::run_reconciliation_worker(
                store, services, elector, enabled,
            )
            .await;
        });
    }
    while let Some(res) = join_set.join_next().await {
        match res {
            Ok(_) => log::info!("Worker completed successfully"),
            Err(e) if e.is_panic() => log::error!("Worker panicked: {:?}", e),
            Err(e) if e.is_cancelled() => log::warn!("Worker was cancelled: {:?}", e),
            Err(e) => log::error!("Worker failed: {:?}", e),
        }
    }

    Ok(())
}

/// The queues no worker of this process reads, which are drained. A core queue without a core
/// consumer is a bug: draining it would drop work, so the workers refuse to start.
fn queues_to_drain(
    consumed: &[PgmqQueue],
    module_consumes: impl Fn(PgmqQueue) -> bool,
) -> Vec<PgmqQueue> {
    PgmqQueue::iter()
        .filter(|queue| !consumed.contains(queue) && !module_consumes(*queue))
        .filter(|queue| match queue.consumer() {
            None => false,
            Some(QueueConsumer::Core) => panic!("core queue {} has no consumer", queue.as_str()),
            Some(QueueConsumer::Integration | QueueConsumer::Module) => true,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn core_queues() -> Vec<PgmqQueue> {
        PgmqQueue::iter()
            .filter(|q| q.consumer() == Some(QueueConsumer::Core))
            .collect()
    }

    #[test]
    fn absent_integrations_and_modules_are_drained_retired_queues_are_not() {
        assert_eq!(
            queues_to_drain(&core_queues(), |_| false),
            vec![
                PgmqQueue::HubspotSync,
                PgmqQueue::PennylaneSync,
                PgmqQueue::DogfoodingSync,
                PgmqQueue::CreditRecompute,
            ]
        );
        let mut all = core_queues();
        all.extend([PgmqQueue::HubspotSync, PgmqQueue::PennylaneSync]);
        assert!(
            queues_to_drain(&all, |q| matches!(
                q,
                PgmqQueue::DogfoodingSync | PgmqQueue::CreditRecompute
            ))
            .is_empty()
        );
    }

    #[test]
    #[should_panic(expected = "core queue webhook_out has no consumer")]
    fn a_core_queue_without_consumer_refuses_to_start() {
        let consumed: Vec<_> = core_queues()
            .into_iter()
            .filter(|q| *q != PgmqQueue::WebhookOut)
            .collect();
        queues_to_drain(&consumed, |_| false);
    }
}
