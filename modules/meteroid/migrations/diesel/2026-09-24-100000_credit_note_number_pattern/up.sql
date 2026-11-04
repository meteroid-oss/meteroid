-- Credit note numbers were hardcoded as CN-{:06}; the default keeps existing sequences unchanged.
ALTER TABLE "invoicing_entity"
    ADD COLUMN "credit_note_number_pattern" TEXT NOT NULL DEFAULT 'CN-{number:6}';
