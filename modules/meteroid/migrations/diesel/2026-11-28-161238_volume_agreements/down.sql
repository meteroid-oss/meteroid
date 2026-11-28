DROP TABLE volume_agreement_basis;
DROP TABLE volume_agreement_member;
DROP TABLE volume_agreement_metric;
DROP TABLE volume_agreement;
ALTER TABLE billable_metric DROP CONSTRAINT billable_metric_id_tenant_key;
ALTER TABLE subscription DROP CONSTRAINT subscription_id_tenant_key;
