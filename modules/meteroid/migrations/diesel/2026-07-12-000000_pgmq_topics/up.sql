-- Topic bindings for outbox fan-out.

-- webhook_out: every event; the worker filters per Svix subscription.
SELECT pgmq.bind_topic('#', 'webhook_out');

-- hubspot_sync
SELECT pgmq.bind_topic('customer.created', 'hubspot_sync');
SELECT pgmq.bind_topic('customer.updated', 'hubspot_sync');
SELECT pgmq.bind_topic('subscription.created', 'hubspot_sync');

-- pennylane_sync
SELECT pgmq.bind_topic('customer.created', 'pennylane_sync');
SELECT pgmq.bind_topic('customer.updated', 'pennylane_sync');
SELECT pgmq.bind_topic('invoice.accounting_pdf_generated', 'pennylane_sync');
SELECT pgmq.bind_topic('invoice.paid', 'pennylane_sync');

-- invoice_orchestration
SELECT pgmq.bind_topic('invoice.accounting_pdf_generated', 'invoice_orchestration');
SELECT pgmq.bind_topic('invoice.finalized', 'invoice_orchestration');
SELECT pgmq.bind_topic('invoice.paid', 'invoice_orchestration');
SELECT pgmq.bind_topic('payment.transaction_received', 'invoice_orchestration');
SELECT pgmq.bind_topic('credit_note.finalized', 'invoice_orchestration');

-- quote_conversion
SELECT pgmq.bind_topic('quote.accepted', 'quote_conversion');

-- dogfooding_sync: worker applies the Production-only filter + transform post-cutover.
SELECT pgmq.bind_topic('invoice.finalized', 'dogfooding_sync');
SELECT pgmq.bind_topic('credit_note.finalized', 'dogfooding_sync');

-- vat_validation: worker re-checks VIES eligibility against the customer as source of truth.
SELECT pgmq.bind_topic('customer.created', 'vat_validation');
SELECT pgmq.bind_topic('customer.updated', 'vat_validation');
