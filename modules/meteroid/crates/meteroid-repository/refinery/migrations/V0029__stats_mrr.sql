

CREATE TYPE "MRRMovementType" as ENUM ('NEW_BUSINESS', 'EXPANSION', 'CONTRACTION', 'CHURN', 'REACTIVATION'); -- , 'INCREMENTAL_USAGE'
CREATE TABLE bi_mrr_movement_log
(
    id              uuid PRIMARY KEY,
    description     text         NOT NULL, -- TODO something structured instead ? So that we can update the wording, but also do updates for a specific case
    movement_type   "MRRMovementType" NOT NULL,
    net_mrr_change   bigint       NOT NULL,
    currency        varchar(3)         NOT NULL,
    created_at       TIMESTAMP(3) NOT NULL DEFAULT CURRENT_TIMESTAMP, -- used to recalculate mrr snapshot if needed
    -- start of the billing period for the line
    applies_to       date NOT NULL,
    invoice_id uuid NOT NULL references invoice on update cascade on delete restrict,
    credit_note_id uuid NULL references credit_note on update cascade on delete restrict,
    plan_version_id uuid NOT NULL references plan_version on update cascade on delete restrict,
    tenant_id uuid NOT NULL references tenant on update cascade on delete restrict
) ;

CREATE INDEX "bi_mrr_movement_log_idx" ON "bi_mrr_movement_log" ("tenant_id", "applies_to");


CREATE TABLE bi_delta_mrr_daily (
    tenant_id UUID NOT NULL,
    plan_version_id UUID NOT NULL,
    date DATE NOT NULL,
    currency TEXT NOT NULL,
    net_mrr_cents BIGINT NOT NULL,
    new_business_cents BIGINT NOT NULL,
    new_business_count int NOT NULL,
    expansion_cents BIGINT NOT NULL,
    expansion_count int NOT NULL,
    contraction_cents BIGINT NOT NULL,
    contraction_count int NOT NULL,
    churn_cents BIGINT NOT NULL,
    churn_count int NOT NULL,
    reactivation_cents BIGINT NOT NULL,
    reactivation_count int NOT NULL,
    PRIMARY KEY(tenant_id, plan_version_id, currency, date)
);



CREATE OR REPLACE FUNCTION fn_update_mrr()
    RETURNS TRIGGER AS $$
BEGIN
    INSERT INTO bi_delta_mrr_daily (tenant_id, plan_version_id, date, currency, net_mrr_cents,  new_business_cents, new_business_count, expansion_cents, expansion_count, contraction_cents, contraction_count, churn_cents, churn_count, reactivation_cents, reactivation_count)
    VALUES (
               NEW.tenant_id,
               NEW.plan_version_id,
               NEW.applies_to,
               NEW.currency,
               NEW.net_mrr_change,
               CASE WHEN NEW.movement_type = 'NEW_BUSINESS' THEN NEW.net_mrr_change ELSE 0 END,
               CASE WHEN NEW.movement_type = 'NEW_BUSINESS' THEN 1 ELSE 0 END,
               CASE WHEN NEW.movement_type = 'EXPANSION' THEN NEW.net_mrr_change ELSE 0 END,
               CASE WHEN NEW.movement_type = 'EXPANSION' THEN 1 ELSE 0 END,
               CASE WHEN NEW.movement_type = 'CONTRACTION' THEN NEW.net_mrr_change ELSE 0 END,
               CASE WHEN NEW.movement_type = 'CONTRACTION' THEN 1 ELSE 0 END,
               CASE WHEN NEW.movement_type = 'CHURN' THEN NEW.net_mrr_change ELSE 0 END,
               CASE WHEN NEW.movement_type = 'CHURN' THEN 1 ELSE 0 END,
               CASE WHEN NEW.movement_type = 'REACTIVATION' THEN NEW.net_mrr_change ELSE 0 END,
               CASE WHEN NEW.movement_type = 'REACTIVATION' THEN 1 ELSE 0 END
           )
    ON CONFLICT (tenant_id, plan_version_id, currency, date) DO UPDATE
        SET
            net_mrr_cents = bi_delta_mrr_daily.net_mrr_cents + EXCLUDED.net_mrr_cents,
            new_business_cents = bi_delta_mrr_daily.new_business_cents + EXCLUDED.new_business_cents,
            new_business_count = bi_delta_mrr_daily.new_business_count + EXCLUDED.new_business_count,
            expansion_cents = bi_delta_mrr_daily.expansion_cents + EXCLUDED.expansion_cents,
            expansion_count = bi_delta_mrr_daily.expansion_count + EXCLUDED.expansion_count,
            contraction_cents = bi_delta_mrr_daily.contraction_cents + EXCLUDED.contraction_cents,
            contraction_count = bi_delta_mrr_daily.contraction_count + EXCLUDED.contraction_count,
            churn_cents = bi_delta_mrr_daily.churn_cents + EXCLUDED.churn_cents,
            churn_count = bi_delta_mrr_daily.churn_count + EXCLUDED.churn_count,
            reactivation_cents = bi_delta_mrr_daily.reactivation_cents + EXCLUDED.reactivation_cents,
            reactivation_count = bi_delta_mrr_daily.reactivation_count + EXCLUDED.reactivation_count;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;


CREATE TRIGGER tr_after_insert_bi_mrr_movement_log
    AFTER INSERT ON bi_mrr_movement_log
    FOR EACH ROW
EXECUTE FUNCTION fn_update_mrr();


CREATE TABLE bi_saas_metrics_monthly (
   id UUID PRIMARY KEY,
   tenant_id UUID NOT NULL,
   month DATE NOT NULL,
   timestamp TIMESTAMP(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
   currency TEXT NOT NULL,
   mrr bigint NOT NULL,
   arr bigint NOT NULL,
   paid_subscriber_count bigint NOT NULL,
   paid_subscriber_churn bigint NOT NULL,
   arpu bigint NOT NULL,
   ltv bigint NOT NULL,
   breakout jsonb NOT NULL
);

