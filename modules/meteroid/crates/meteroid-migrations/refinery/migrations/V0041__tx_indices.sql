update customer set billing_config = '"Manual"'::jsonb where billing_config is null;
alter table customer alter column billing_config drop NOT NULL;
alter table customer_balance_pending_tx alter column invoice_id drop NOT NULL;

alter table customer_balance_pending_tx
  add constraint customer_balance_pending_tx_amount_positive check (customer_balance_pending_tx.amount_cents > 0);

create unique index customer_balance_pending_tx_invoice_id on customer_balance_pending_tx(invoice_id);
create unique index customer_balance_tx_invoice_id on customer_balance_tx(invoice_id);
