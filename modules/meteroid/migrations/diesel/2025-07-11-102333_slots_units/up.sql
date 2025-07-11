
ALTER TABLE slot_transaction ADD COLUMN unit VARCHAR(255) NOT NULL DEFAULT 'seats';
ALTER TABLE slot_transaction DROP COLUMN "price_component_id";

