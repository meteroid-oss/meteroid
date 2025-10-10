use crate::StoreResult;
use crate::domain::{Customer, CustomerBuyCredits, DetailedInvoice, LineItem};
use crate::errors::StoreError;
use crate::repositories::InvoiceInterface;
use crate::services::Services;
use crate::store::PgConn;
use crate::utils::local_id::{IdType, LocalId};
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_models::customer_balance_txs::CustomerBalancePendingTxRowNew;
use diesel_models::customers::CustomerRow;
use error_stack::Report;
use uuid::Uuid;

impl Services {
    pub(crate) async fn buy_customer_credits(
        &self,
        conn: &mut PgConn,
        req: CustomerBuyCredits,
    ) -> StoreResult<DetailedInvoice> {
        let invoice = self
            .store
            .transaction_with(conn, |conn| {
                async move {
                    let now = chrono::Utc::now().naive_utc();

                    let customer: Customer =
                        CustomerRow::find_by_id(conn, &req.customer_id, &req.tenant_id)
                            .await
                            .map_err(Into::<Report<StoreError>>::into)
                            .and_then(TryInto::try_into)?;

                    let currency = customer.currency.clone();
                    let line_items = vec![LineItem {
                        local_id: LocalId::generate_for(IdType::Other),
                        name: "Purchase credits".into(),
                        amount_total: req.cents,
                        amount_subtotal: req.cents,
                        taxable_amount: req.cents,
                        tax_amount: 0,
                        unit_price: Some(req.cents.into()),
                        quantity: Some(1.into()),
                        start_date: now.date(),
                        end_date: now.date(),
                        sub_lines: vec![],
                        is_prorated: false,
                        price_component_id: None,
                        sub_component_id: None,
                        sub_add_on_id: None,
                        product_id: None,
                        metric_id: None,
                        description: None,
                        tax_rate: Default::default(),
                        group_by_dimensions: None,
                    }];

                    let invoice = self
                        .create_oneoff_draft_invoice(
                            conn,
                            req.tenant_id,
                            now.date(),
                            line_items,
                            &customer,
                            currency,
                            None,
                            None,
                        )
                        .await?
                        .ok_or(
                            Report::new(StoreError::BillingError)
                                .attach("Failed to create one-off draft invoice"),
                        )?;

                    let tx = CustomerBalancePendingTxRowNew {
                        id: Uuid::now_v7(),
                        amount_cents: req.cents,
                        note: req.notes,
                        invoice_id: invoice.id,
                        tenant_id: req.tenant_id,
                        customer_id: req.customer_id,
                        tx_id: None,
                        created_by: req.created_by,
                    };

                    tx.insert(conn)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;

                    Ok(invoice)
                }
                .scope_boxed()
            })
            .await?;

        self.store
            .get_detailed_invoice_by_id(req.tenant_id, invoice.id)
            .await
    }
}
