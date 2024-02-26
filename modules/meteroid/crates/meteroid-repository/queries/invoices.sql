--: ListInvoice(days_until_due?, amount_cents?)
--: Invoice(days_until_due?, amount_cents?, last_issue_attempt_at?,last_issue_error?)
--: DetailedInvoice(days_until_due?, amount_cents?, updated_at?, last_issue_attempt_at?,last_issue_error?)

--! create_invoice (amount_cents?) : Invoice
INSERT INTO invoice (id,
                     status,
                     invoicing_provider,
                     invoice_date,
                     tenant_id,
                     customer_id,
                     subscription_id,
                     plan_version_id,
                     currency,
                     days_until_due,
                     line_items,
                     amount_cents,
                     finalized_at)
VALUES (:id,
        :status::"InvoiceStatusEnum",
        :invoicing_provider,
        :invoice_date,
        :tenant_id,
        :customer_id,
        :subscription_id,
        :plan_version_id,
        :currency,
        :days_until_due,
        :line_items,
        :amount_cents,
        CASE WHEN :status::"InvoiceStatusEnum" = 'FINALIZED' THEN NOW() else null END
       )
RETURNING id, status, invoicing_provider, invoice_date, tenant_id, customer_id, subscription_id, plan_version_id, currency, days_until_due, line_items, amount_cents, issued, issue_attempts,last_issue_attempt_at,last_issue_error;


--! update_invoice_status
UPDATE invoice
SET status     = :status::"InvoiceStatusEnum",
    updated_at = NOW(),
    finalized_at = CASE WHEN :status::"InvoiceStatusEnum" = 'FINALIZED' THEN NOW() ELSE finalized_at END
WHERE id = :id
  AND status NOT IN ('FINALIZED', 'VOID');

--! update_invoice_external_status
UPDATE invoice
SET external_status = :external_status,
    updated_at      = NOW()
WHERE id = :id
RETURNING id, external_status;

--! update_invoice_lines
UPDATE invoice
SET line_items      = :line_items,
    updated_at      = NOW(),
    data_updated_at = NOW()
WHERE id = :id
  AND status NOT IN ('FINALIZED', 'VOID');


--! patch_invoice (status?, invoicing_provider?, invoice_date?, currency?, line_items?) : Invoice
UPDATE invoice
SET status             = COALESCE(:status, status),
    invoicing_provider = COALESCE(:invoicing_provider, invoicing_provider),
    invoice_date       = COALESCE(:invoice_date, invoice_date),
    currency           = COALESCE(:currency, currency),
    days_until_due     = COALESCE(:days_until_due, days_until_due),
    line_items         = COALESCE(:line_items, line_items),
    amount_cents       = COALESCE(:amount_cents, amount_cents),
    updated_at         = NOW(),
    finalized_at = CASE WHEN :status::"InvoiceStatusEnum" = 'FINALIZED' THEN NOW() ELSE finalized_at END
WHERE id = :id
  AND status NOT IN ('FINALIZED', 'VOID')
RETURNING id, status, invoicing_provider, invoice_date, tenant_id, customer_id, subscription_id, plan_version_id, currency, days_until_due, line_items, amount_cents, issued, issue_attempts,last_issue_attempt_at,last_issue_error;

-- Update the invoices with 'DRAFT' status where the end date has passed but the grace period is not over (or won't be in the next 5 minutes)
--! update_pending_finalization_invoices
UPDATE invoice
SET status     = 'PENDING',
    updated_at = NOW()
FROM invoicing_config
WHERE invoice.tenant_id = invoicing_config.tenant_id
  AND invoice.status = 'DRAFT'
  AND invoice.invoice_date < NOW()
  AND NOW() <= (invoice.invoice_date + interval '1 hour' * invoicing_config.grace_period_hours)
RETURNING invoice.id, invoice.status;

-- get invoices not voided/finalized whose grace period is over
--! get_invoices_to_finalize : Invoice
SELECT invoice.id,
       invoice.status,
       invoice.invoicing_provider,
       invoice.invoice_date,
       invoice.tenant_id,
       invoice.customer_id,
       invoice.subscription_id,
       invoice.plan_version_id,
       invoice.currency,
       invoice.days_until_due,
       invoice.line_items,
       invoice.amount_cents,
       invoice.issued,
       invoice.issue_attempts,
       invoice.last_issue_attempt_at,
       invoice.last_issue_error
FROM invoice
         JOIN invoicing_config ON invoice.tenant_id = invoicing_config.tenant_id
WHERE invoice.status NOT IN ('VOID', 'FINALIZED')
  AND NOW() > (invoice.invoice_date + interval '1 hour' * invoicing_config.grace_period_hours);
-- TODO calculate that upfront, and sort by it

-- get invoices not voided/finalized whose data was not updated in the last X hours
--! get_outdated_invoices : Invoice
SELECT invoice.id,
       invoice.status,
       invoice.invoicing_provider,
       invoice.invoice_date,
       invoice.tenant_id,
       invoice.customer_id,
       invoice.subscription_id,
       invoice.plan_version_id,
       invoice.currency,
       invoice.days_until_due,
       invoice.line_items,
       invoice.amount_cents,
       invoice.issued,
       invoice.issue_attempts,
       invoice.last_issue_attempt_at,
       invoice.last_issue_error
FROM invoice
WHERE invoice.status NOT IN ('VOID', 'FINALIZED')
  AND (
    invoice.data_updated_at IS NULL
        OR invoice.data_updated_at < NOW() -
                                     interval '1 hour' -- TODO configurable, per org plan (via invoicing_config) & store skew metric for alerting
    )
ORDER BY invoice.data_updated_at IS NULL DESC,
         invoice.data_updated_at ASC;


-- get finalized invoices to be issued
--! get_invoices_to_issue(issue_max_attempts) : Invoice
SELECT id,
       status,
       invoicing_provider,
       invoice_date,
       tenant_id,
       customer_id,
       subscription_id,
       plan_version_id,
       currency,
       days_until_due,
       line_items,
       amount_cents,
       issued,
       issue_attempts,
       last_issue_attempt_at,
       last_issue_error
FROM invoice
WHERE status = 'FINALIZED'
  AND issued = false
  AND issue_attempts < :issue_max_attempts;

--! list_tenant_invoices (search?, status?, customer_id?) : ListInvoice
SELECT invoice.id,
       invoice.status,
       invoice.invoicing_provider,
       invoice.created_at,
       invoice.invoice_date,
       invoice.customer_id,
       invoice.subscription_id,
       invoice.currency,
       invoice.days_until_due,
       invoice.amount_cents,
       customer.name    AS customer_name,
       COUNT(*) OVER () AS total_count
FROM invoice
         JOIN customer ON customer_id = customer.id
WHERE invoice.tenant_id = :tenant_id
  AND (:status :: "InvoiceStatusEnum" IS NULL OR invoice.status = :status)
  AND (:search :: TEXT IS NULL OR customer.name ILIKE '%' || :search || '%')
  AND (:customer_id :: UUID IS NULL OR customer_id = :customer_id)
ORDER BY CASE
             WHEN :order_by = 'DATE_DESC' THEN invoice.created_at
             END DESC,
         CASE
             WHEN :order_by = 'DATE_ASC' THEN invoice.created_at
             END ASC,
         CASE
             WHEN :order_by = 'ID_DESC' THEN invoice.invoice_id
             END DESC,
         CASE
             WHEN :order_by = 'ID_ASC' THEN invoice.invoice_id
             END ASC
LIMIT :limit OFFSET :offset;

-- Update the invoice as issued
--! update_invoice_issue_success(id, issue_attempts)
UPDATE invoice
SET issued                = true,
    issue_attempts        = :issue_attempts,
    last_issue_attempt_at = NOW(),
    updated_at            = NOW()
WHERE id = :id
  AND status = 'FINALIZED'
  AND issued = false;

-- Update the invoice with issue errors
--! update_invoice_issue_error(id, issue_attempts, last_issue_error)
UPDATE invoice
SET issue_attempts        = :issue_attempts,
    last_issue_attempt_at = NOW(),
    updated_at            = NOW(),
    last_issue_error      = :last_issue_error
WHERE id = :id
  AND status = 'FINALIZED'
  AND issued = false;

--! invoice_by_id: Invoice
SELECT id,
       status,
       invoicing_provider,
       invoice_date,
       tenant_id,
       customer_id,
       subscription_id,
       plan_version_id,
       currency,
       days_until_due,
       line_items,
       amount_cents,
       issued,
       issue_attempts,
       last_issue_attempt_at,
       last_issue_error
FROM invoice
WHERE id = :id;

--! get_tenant_invoice_by_id: DetailedInvoice
SELECT invoice.id,
       invoice.status,
       invoice.invoicing_provider,
       invoice.created_at,
       invoice.updated_at,
       invoice.invoice_date,
       invoice.customer_id,
       invoice.subscription_id,
       invoice.currency,
       invoice.days_until_due,
       invoice.issued,
       invoice.issue_attempts,
       invoice.last_issue_attempt_at,
       invoice.last_issue_error,
       invoice.amount_cents,
       customer.name        AS customer_name,
       plan.name            AS plan_name,
       plan.external_id     AS plan_external_id,
       plan_version.version AS plan_version
FROM invoice
         JOIN customer ON customer_id = customer.id
         JOIN subscription ON subscription_id = subscription.id
         JOIN plan_version ON subscription.plan_version_id = plan_version.id
         JOIN plan ON plan_version.plan_id = plan.id
WHERE invoice.id = :id
  AND invoice.tenant_id = :tenant_id;
