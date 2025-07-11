
TRUNCATE slot_transaction;
ALTER TABLE slot_transaction DROP COLUMN unit;
ALTER TABLE slot_transaction ADD COLUMN "price_component_id" UUID NOT NULL REFERENCES price_component(id);
