-- The QUOTE_CERTIFICATE_PDF value of "StoredObjectKindEnum" stays: Postgres cannot remove an enum value.
DROP TABLE quote_comment;
DROP TABLE quote_approval;
DROP TABLE quote_approval_rule;
DROP TABLE quote_signature_certificate;
DROP TABLE quote_signature_otp;
DROP TABLE quote_evidence_event;
DROP TABLE quote_draft;
DROP TABLE quote_revision;

ALTER TABLE quote_signature
    DROP COLUMN signer_role,
    DROP COLUMN revision,
    DROP COLUMN document_hash,
    DROP COLUMN consent_text,
    DROP COLUMN otp_verified_at;

ALTER TABLE quote_add_on
    DROP COLUMN description,
    DROP COLUMN sort_order,
    DROP COLUMN discount_percent,
    DROP COLUMN list_fee;

ALTER TABLE quote_component
    DROP COLUMN description,
    DROP COLUMN sort_order,
    DROP COLUMN discount_percent,
    DROP COLUMN list_fee;

ALTER TABLE quote
    DROP COLUMN revision,
    DROP COLUMN document_hash,
    DROP COLUMN published_at,
    DROP COLUMN first_viewed_at,
    DROP COLUMN owner_id,
    DROP COLUMN template_id,
    DROP COLUMN decline_reason,
    DROP COLUMN display_options,
    DROP COLUMN sections,
    DROP COLUMN signature_level,
    DROP COLUMN consent_text,
    DROP COLUMN countersigners,
    DROP COLUMN approval_status,
    DROP COLUMN hubspot_deal_id,
    DROP COLUMN max_discount_percent,
    DROP COLUMN changes_requested_at;

DROP TABLE quote_template;

DROP TYPE "QuoteApprovalStatusEnum";
DROP TYPE "QuoteSignerRoleEnum";
DROP TYPE "QuoteSignatureLevelEnum";
