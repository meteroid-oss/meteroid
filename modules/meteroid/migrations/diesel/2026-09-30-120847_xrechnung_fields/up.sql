-- XRechnung (BR-DE-2, BR-DE-5..7) requires a seller contact on every invoice. The
-- email doubles as the seller's electronic address (BT-34, scheme EM).
ALTER TABLE invoicing_entity
    ADD COLUMN contact_name TEXT,
    ADD COLUMN contact_phone TEXT,
    ADD COLUMN contact_email TEXT;

-- BT-10: the reference the buyer routes an invoice by. For a German public body it is
-- the Leitweg-ID, and XRechnung requires one (BR-DE-15).
ALTER TABLE customer
    ADD COLUMN buyer_reference TEXT;
