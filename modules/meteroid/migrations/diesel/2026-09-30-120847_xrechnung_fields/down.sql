ALTER TABLE customer
    DROP COLUMN buyer_reference;

ALTER TABLE invoicing_entity
    DROP COLUMN contact_name,
    DROP COLUMN contact_phone,
    DROP COLUMN contact_email;
