use futures::stream;
use meteroid_store::domain::{Outbox, OutboxEvent};
use meteroid_store::repositories::outbox::OutboxInterface;
use meteroid_store::Store;
use std::collections::HashMap;
use std::sync::Arc;

use crate::services::invoice_rendering::{GenerateResult, PdfRenderingService};
use futures::stream::StreamExt;
use itertools::Itertools;
use tokio::time::Duration;
use uuid::Uuid;
/*

In this iteration, the outbox tx table is read by a separate poller for each event type, and processed synchronously.
It locks the rows for the duration of the processing (allowing multiple instances), and retries on failure.

We should add kafka to decouple the scaling of (some) consumers (in the pdf case, we need a single consumer per gotenberg instance)

 */

pub struct InvoiceFinalizedOutboxWorker {
    pdf_service: PdfRenderingService,
    store: Arc<Store>,
}

impl InvoiceFinalizedOutboxWorker {
    pub fn new(pdf_service: PdfRenderingService, store: Arc<Store>) -> Self {
        Self { pdf_service, store }
    }

    pub async fn run(&self) {
        loop {
            let outbox = match self
                .store
                .claim_outbox_entries(OutboxEvent::InvoiceFinalized, 10)
                .await
            {
                Ok(entries) => entries,
                Err(e) => {
                    tracing::error!("Error while claiming outbox entries: {}", e);
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    continue;
                }
            };

            if outbox.is_empty() {
                tokio::time::sleep(Duration::from_secs(5)).await;
                continue;
            }

            let outbox_map: HashMap<_, _> = outbox
                .iter()
                .map(|entry| (entry.resource_id, entry))
                .collect();

            match self
                .pdf_service
                .generate_pdfs(outbox_map.keys().cloned().collect())
                .await
            {
                Ok(results) => self.process_results(&outbox_map, results).await,
                Err(e) => self.mark_all_as_failed(&outbox, e.to_string()).await,
            }
        }
    }

    async fn process_results(
        &self,
        outbox_map: &HashMap<Uuid, &Outbox>,
        results: Vec<GenerateResult>,
    ) {
        let (successes, failures): (Vec<_>, Vec<_>) =
            results.into_iter().partition_map(|result| match result {
                GenerateResult::Success { invoice_id, .. } => itertools::Either::Left(invoice_id),
                GenerateResult::Failure { invoice_id, error } => {
                    itertools::Either::Right((invoice_id, error))
                }
            });

        if !successes.is_empty() {
            let success_entry_ids: Vec<_> = successes
                .iter()
                .filter_map(|id| outbox_map.get(id).map(|entry| entry.id))
                .collect();

            if let Err(e) = self
                .store
                .mark_outbox_entries_as_completed(success_entry_ids)
                .await
            {
                tracing::error!("Error while saving successful outbox responses: {}", e);
            }
        }

        stream::iter(failures)
            .for_each(|(invoice_id, error)| async move {
                if let Some(&entry) = outbox_map.get(&invoice_id) {
                    if let Err(e) = self
                        .store
                        .mark_outbox_entry_as_failed(entry.id, error)
                        .await
                    {
                        tracing::error!("Error while saving failed outbox response: {}", e);
                    }
                }
            })
            .await;
    }

    async fn mark_all_as_failed(&self, outbox: &[Outbox], error: String) {
        if let Err(e) = self
            .store
            .mark_outbox_entries_as_failed(outbox.iter().map(|entry| entry.id).collect(), error)
            .await
        {
            tracing::error!("Error while marking all outbox entries as failed: {}", e);
        }
    }
}
