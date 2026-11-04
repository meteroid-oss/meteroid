SELECT pgmq.unbind_topic('#', 'webhook_out');

SELECT pgmq.unbind_topic('customer.created', 'hubspot_sync');
SELECT pgmq.unbind_topic('customer.updated', 'hubspot_sync');
SELECT pgmq.unbind_topic('subscription.created', 'hubspot_sync');

SELECT pgmq.unbind_topic('customer.created', 'pennylane_sync');
SELECT pgmq.unbind_topic('customer.updated', 'pennylane_sync');
SELECT pgmq.unbind_topic('invoice.accounting_pdf_generated', 'pennylane_sync');
SELECT pgmq.unbind_topic('invoice.paid', 'pennylane_sync');

SELECT pgmq.unbind_topic('invoice.accounting_pdf_generated', 'invoice_orchestration');
SELECT pgmq.unbind_topic('invoice.finalized', 'invoice_orchestration');
SELECT pgmq.unbind_topic('invoice.paid', 'invoice_orchestration');
SELECT pgmq.unbind_topic('payment.transaction_received', 'invoice_orchestration');
SELECT pgmq.unbind_topic('credit_note.finalized', 'invoice_orchestration');

SELECT pgmq.unbind_topic('quote.accepted', 'quote_conversion');

SELECT pgmq.unbind_topic('invoice.finalized', 'dogfooding_sync');
SELECT pgmq.unbind_topic('credit_note.finalized', 'dogfooding_sync');

SELECT pgmq.unbind_topic('customer.created', 'vat_validation');
SELECT pgmq.unbind_topic('customer.updated', 'vat_validation');
