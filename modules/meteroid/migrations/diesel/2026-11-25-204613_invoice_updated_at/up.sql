-- Public API v2 (ADR-0013 §12): when an invoice last changed, which `GET /api/v2/invoices` filters
-- on (`updated_after`). `invoice.updated_at` was written by some paths only (status, settlement,
-- custom properties); a draft's identity, annexes or memo edited moved nothing. Every write of the
-- row now sets it, the columns the engine and the store keep for themselves aside.
-- A long transaction of a serving pod fails this run instead of queueing every query behind it;
-- the next start retries.
SET LOCAL lock_timeout = '5s';

-- A draft whose lines were refreshed after its `updated_at` was last written takes that time, so
-- that `updated_at` alone says when any row last changed and a refresh need not count. Drafts only:
-- an issued invoice's two stamps can differ by microseconds, and rewriting every issued invoice
-- would re-stamp them all for the analytics projector. The drafts are a few per subscription at
-- most (about 20,000 subscriptions in the largest installation), so a few seconds at worst.
UPDATE invoice
SET updated_at = data_updated_at AT TIME ZONE 'UTC'
WHERE status = 'DRAFT'
  AND data_updated_at AT TIME ZONE 'UTC' > coalesce(updated_at, created_at);

-- What is bookkeeping, writing which changes nothing a reader of the invoice sees:
--   updated_at                  the column itself (a path setting it keeps its value);
--   data_updated_at             when a draft's lines were last computed: the background refresh
--                               stamps it hourly while a draft waits, whether or not they moved;
--                               lines that moved count through `line_items` and the totals;
--   store_committed_at, store_xid
--                               the analytics projector's markers, re-stamped by their own
--                               triggers on every write;
--   conn_meta                   the ids of the invoice in connected accounting tools;
--   next_payment_attempt_at, auto_charge_requested_at
--                               the payment retries and the automatic charge the engine schedules;
--   finalize_after, delivery_held_at
--                               when the engine may finalize the draft and send it.
-- Anything else is an edit: lines and totals, status, payment, dates, the customer's and the
-- seller's details, annexes, memo, reference, purchase order, custom properties, files, the
-- e-invoicing status and report.
--
-- No backfill beyond the refreshed rows above (it would rewrite the table and re-stamp every row
-- for the projector): a row unchanged since reads as changed at the later of its `updated_at` and
-- its creation. A BEFORE trigger on the row being written takes no lock of its own. The stamp is
-- the clock when the row is written (`clock_timestamp()`, not `now()`, the transaction's start):
-- the change becomes visible when its transaction commits, at most that transaction's remaining
-- length later.
CREATE FUNCTION invoice_touch() RETURNS trigger
    LANGUAGE plpgsql AS
$$
DECLARE
    bookkeeping TEXT[] := ARRAY [
        'updated_at', 'data_updated_at', 'store_committed_at', 'store_xid', 'conn_meta',
        'next_payment_attempt_at', 'auto_charge_requested_at', 'finalize_after', 'delivery_held_at'];
BEGIN
    IF (to_jsonb(NEW) - bookkeeping) IS DISTINCT FROM (to_jsonb(OLD) - bookkeeping) THEN
        NEW.updated_at := clock_timestamp();
    END IF;
    RETURN NEW;
END
$$;

CREATE TRIGGER trg_invoice_touch
    BEFORE UPDATE ON invoice
    FOR EACH ROW EXECUTE FUNCTION invoice_touch();
