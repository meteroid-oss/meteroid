-- What a subscription cancelled at once consumed after its period's own invoice, billed apart
-- from that invoice's (subscription, period start) key. Not usable before this commits: the
-- index keyed on it is the next migration.
ALTER TYPE "InvoiceType" ADD VALUE IF NOT EXISTS 'FINAL';
