CREATE SCHEMA IF NOT EXISTS pgmq;
CREATE EXTENSION IF NOT EXISTS pgmq WITH SCHEMA pgmq;

SELECT pgmq.create('outbox_event');
SELECT pgmq.create('invoice_pdf_request');
SELECT pgmq.create('webhook_out');
