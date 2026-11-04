-- Make `organization` a clean hard-delete cascade root: convert every tenant_id
-- / organization_id trunk FK (plus two orphan child edges) to ON DELETE CASCADE
-- so a single `DELETE FROM organization` wipes the subtree. Intra-tenant FKs keep
-- their RESTRICT/NO ACTION semantics; the changed edges only fire on a full
-- tenant/organization delete.

-- 1. Convert existing tenant_id / organization_id FKs (and the two orphan child
--    edges slot_transaction->subscription, webhook_in_event->connector) to CASCADE.
ALTER TABLE public.add_on DROP CONSTRAINT add_on_tenant_id_fkey;
ALTER TABLE public.add_on ADD CONSTRAINT add_on_tenant_id_fkey FOREIGN KEY (tenant_id) REFERENCES tenant(id) ON UPDATE CASCADE ON DELETE CASCADE;
ALTER TABLE public.api_token DROP CONSTRAINT api_token_tenant_id_fkey;
ALTER TABLE public.api_token ADD CONSTRAINT api_token_tenant_id_fkey FOREIGN KEY (tenant_id) REFERENCES tenant(id) ON UPDATE CASCADE ON DELETE CASCADE;
ALTER TABLE public.bank_account DROP CONSTRAINT bank_account_tenant_id_fkey;
ALTER TABLE public.bank_account ADD CONSTRAINT bank_account_tenant_id_fkey FOREIGN KEY (tenant_id) REFERENCES tenant(id) ON DELETE CASCADE;
ALTER TABLE public.batch_job DROP CONSTRAINT batch_job_tenant_id_fkey;
ALTER TABLE public.batch_job ADD CONSTRAINT batch_job_tenant_id_fkey FOREIGN KEY (tenant_id) REFERENCES tenant(id) ON DELETE CASCADE;
ALTER TABLE public.batch_job_chunk DROP CONSTRAINT batch_job_chunk_tenant_id_fkey;
ALTER TABLE public.batch_job_chunk ADD CONSTRAINT batch_job_chunk_tenant_id_fkey FOREIGN KEY (tenant_id) REFERENCES tenant(id) ON DELETE CASCADE;
ALTER TABLE public.batch_job_entity DROP CONSTRAINT batch_job_entity_tenant_id_fkey;
ALTER TABLE public.batch_job_entity ADD CONSTRAINT batch_job_entity_tenant_id_fkey FOREIGN KEY (tenant_id) REFERENCES tenant(id) ON DELETE CASCADE;
ALTER TABLE public.batch_job_item_failure DROP CONSTRAINT batch_job_item_failure_tenant_id_fkey;
ALTER TABLE public.batch_job_item_failure ADD CONSTRAINT batch_job_item_failure_tenant_id_fkey FOREIGN KEY (tenant_id) REFERENCES tenant(id) ON DELETE CASCADE;
ALTER TABLE public.bi_mrr_movement_log DROP CONSTRAINT bi_mrr_movement_log_tenant_id_fkey;
ALTER TABLE public.bi_mrr_movement_log ADD CONSTRAINT bi_mrr_movement_log_tenant_id_fkey FOREIGN KEY (tenant_id) REFERENCES tenant(id) ON UPDATE CASCADE ON DELETE CASCADE;
ALTER TABLE public.billable_metric DROP CONSTRAINT billable_metric_tenant_id_fkey;
ALTER TABLE public.billable_metric ADD CONSTRAINT billable_metric_tenant_id_fkey FOREIGN KEY (tenant_id) REFERENCES tenant(id) ON UPDATE CASCADE ON DELETE CASCADE;
ALTER TABLE public.checkout_session DROP CONSTRAINT checkout_session_tenant_id_fkey;
ALTER TABLE public.checkout_session ADD CONSTRAINT checkout_session_tenant_id_fkey FOREIGN KEY (tenant_id) REFERENCES tenant(id) ON DELETE CASCADE;
ALTER TABLE public.connect_access_token DROP CONSTRAINT connect_access_token_connected_organization_id_fkey;
ALTER TABLE public.connect_access_token ADD CONSTRAINT connect_access_token_connected_organization_id_fkey FOREIGN KEY (connected_organization_id) REFERENCES organization(id) ON DELETE CASCADE;
ALTER TABLE public.connect_authorization_code DROP CONSTRAINT connect_authorization_code_connected_organization_id_fkey;
ALTER TABLE public.connect_authorization_code ADD CONSTRAINT connect_authorization_code_connected_organization_id_fkey FOREIGN KEY (connected_organization_id) REFERENCES organization(id) ON DELETE CASCADE;
ALTER TABLE public.connect_connected_account DROP CONSTRAINT connect_connected_account_connected_organization_id_fkey;
ALTER TABLE public.connect_connected_account ADD CONSTRAINT connect_connected_account_connected_organization_id_fkey FOREIGN KEY (connected_organization_id) REFERENCES organization(id) ON DELETE CASCADE;
ALTER TABLE public.connect_connected_account DROP CONSTRAINT connect_connected_account_connected_tenant_id_fkey;
ALTER TABLE public.connect_connected_account ADD CONSTRAINT connect_connected_account_connected_tenant_id_fkey FOREIGN KEY (connected_tenant_id) REFERENCES tenant(id) ON DELETE CASCADE;
ALTER TABLE public.connect_connected_account DROP CONSTRAINT connect_connected_account_platform_organization_id_fkey;
ALTER TABLE public.connect_connected_account ADD CONSTRAINT connect_connected_account_platform_organization_id_fkey FOREIGN KEY (platform_organization_id) REFERENCES organization(id) ON DELETE CASCADE;
ALTER TABLE public.connect_oauth_app DROP CONSTRAINT connect_oauth_app_organization_id_fkey;
ALTER TABLE public.connect_oauth_app ADD CONSTRAINT connect_oauth_app_organization_id_fkey FOREIGN KEY (organization_id) REFERENCES organization(id) ON DELETE CASCADE;
ALTER TABLE public.connect_platform_settings DROP CONSTRAINT connect_platform_settings_organization_id_fkey;
ALTER TABLE public.connect_platform_settings ADD CONSTRAINT connect_platform_settings_organization_id_fkey FOREIGN KEY (organization_id) REFERENCES organization(id) ON DELETE CASCADE;
ALTER TABLE public.coupon DROP CONSTRAINT coupon_tenant_id_fkey;
ALTER TABLE public.coupon ADD CONSTRAINT coupon_tenant_id_fkey FOREIGN KEY (tenant_id) REFERENCES tenant(id) ON UPDATE CASCADE ON DELETE CASCADE;
ALTER TABLE public.credit_note DROP CONSTRAINT credit_note_tenant_id_fkey;
ALTER TABLE public.credit_note ADD CONSTRAINT credit_note_tenant_id_fkey FOREIGN KEY (tenant_id) REFERENCES tenant(id) ON UPDATE CASCADE ON DELETE CASCADE;
ALTER TABLE public.customer DROP CONSTRAINT customer_tenant_id_fkey;
ALTER TABLE public.customer ADD CONSTRAINT customer_tenant_id_fkey FOREIGN KEY (tenant_id) REFERENCES tenant(id) ON UPDATE CASCADE ON DELETE CASCADE;
ALTER TABLE public.customer_balance_pending_tx DROP CONSTRAINT customer_balance_pending_tx_tenant_id_fkey;
ALTER TABLE public.customer_balance_pending_tx ADD CONSTRAINT customer_balance_pending_tx_tenant_id_fkey FOREIGN KEY (tenant_id) REFERENCES tenant(id) ON UPDATE CASCADE ON DELETE CASCADE;
ALTER TABLE public.customer_balance_tx DROP CONSTRAINT customer_balance_tx_tenant_id_fkey;
ALTER TABLE public.customer_balance_tx ADD CONSTRAINT customer_balance_tx_tenant_id_fkey FOREIGN KEY (tenant_id) REFERENCES tenant(id) ON UPDATE CASCADE ON DELETE CASCADE;
ALTER TABLE public.customer_payment_method DROP CONSTRAINT customer_payment_method_tenant_id_fkey;
ALTER TABLE public.customer_payment_method ADD CONSTRAINT customer_payment_method_tenant_id_fkey FOREIGN KEY (tenant_id) REFERENCES tenant(id) ON DELETE CASCADE;
ALTER TABLE public.dead_letter_message DROP CONSTRAINT dead_letter_message_tenant_id_fkey;
ALTER TABLE public.dead_letter_message ADD CONSTRAINT dead_letter_message_tenant_id_fkey FOREIGN KEY (tenant_id) REFERENCES tenant(id) ON DELETE CASCADE;
ALTER TABLE public.invoice DROP CONSTRAINT invoice_tenant_id_fkey;
ALTER TABLE public.invoice ADD CONSTRAINT invoice_tenant_id_fkey FOREIGN KEY (tenant_id) REFERENCES tenant(id) ON UPDATE CASCADE ON DELETE CASCADE;
ALTER TABLE public.invoice_annex_template DROP CONSTRAINT invoice_annex_template_tenant_id_fkey;
ALTER TABLE public.invoice_annex_template ADD CONSTRAINT invoice_annex_template_tenant_id_fkey FOREIGN KEY (tenant_id) REFERENCES tenant(id) ON DELETE CASCADE;
ALTER TABLE public.organization_invite DROP CONSTRAINT organization_invite_organization_id_fkey;
ALTER TABLE public.organization_invite ADD CONSTRAINT organization_invite_organization_id_fkey FOREIGN KEY (organization_id) REFERENCES organization(id) ON DELETE CASCADE;
ALTER TABLE public.organization_member DROP CONSTRAINT organization_member_organization_id_fkey;
ALTER TABLE public.organization_member ADD CONSTRAINT organization_member_organization_id_fkey FOREIGN KEY (organization_id) REFERENCES organization(id) ON UPDATE CASCADE ON DELETE CASCADE;
ALTER TABLE public.role DROP CONSTRAINT role_organization_id_fkey;
ALTER TABLE public.role ADD CONSTRAINT role_organization_id_fkey FOREIGN KEY (organization_id) REFERENCES organization(id) ON DELETE CASCADE;
ALTER TABLE public.payment_transaction DROP CONSTRAINT payment_transaction_tenant_id_fkey;
ALTER TABLE public.payment_transaction ADD CONSTRAINT payment_transaction_tenant_id_fkey FOREIGN KEY (tenant_id) REFERENCES tenant(id) ON DELETE CASCADE;
ALTER TABLE public.plan DROP CONSTRAINT plan_tenant_id_fkey;
ALTER TABLE public.plan ADD CONSTRAINT plan_tenant_id_fkey FOREIGN KEY (tenant_id) REFERENCES tenant(id) ON UPDATE CASCADE ON DELETE CASCADE;
ALTER TABLE public.plan_version_add_on DROP CONSTRAINT plan_version_add_on_tenant_id_fkey;
ALTER TABLE public.plan_version_add_on ADD CONSTRAINT plan_version_add_on_tenant_id_fkey FOREIGN KEY (tenant_id) REFERENCES tenant(id) ON DELETE CASCADE;
ALTER TABLE public.plan_version_annex_template DROP CONSTRAINT plan_version_annex_template_tenant_id_fkey;
ALTER TABLE public.plan_version_annex_template ADD CONSTRAINT plan_version_annex_template_tenant_id_fkey FOREIGN KEY (tenant_id) REFERENCES tenant(id) ON DELETE CASCADE;
ALTER TABLE public.price DROP CONSTRAINT price_tenant_id_fkey;
ALTER TABLE public.price ADD CONSTRAINT price_tenant_id_fkey FOREIGN KEY (tenant_id) REFERENCES tenant(id) ON DELETE CASCADE;
ALTER TABLE public.product DROP CONSTRAINT product_tenant_id_fkey;
ALTER TABLE public.product ADD CONSTRAINT product_tenant_id_fkey FOREIGN KEY (tenant_id) REFERENCES tenant(id) ON UPDATE CASCADE ON DELETE CASCADE;
ALTER TABLE public.product_family DROP CONSTRAINT product_family_tenant_id_fkey;
ALTER TABLE public.product_family ADD CONSTRAINT product_family_tenant_id_fkey FOREIGN KEY (tenant_id) REFERENCES tenant(id) ON UPDATE CASCADE ON DELETE CASCADE;
ALTER TABLE public.quote DROP CONSTRAINT quote_tenant_id_fkey;
ALTER TABLE public.quote ADD CONSTRAINT quote_tenant_id_fkey FOREIGN KEY (tenant_id) REFERENCES tenant(id) ON DELETE CASCADE;
ALTER TABLE public.slot_transaction DROP CONSTRAINT slot_transaction_subscription_id_fkey;
ALTER TABLE public.slot_transaction ADD CONSTRAINT slot_transaction_subscription_id_fkey FOREIGN KEY (subscription_id) REFERENCES subscription(id) ON UPDATE CASCADE ON DELETE CASCADE;
ALTER TABLE public.subscription DROP CONSTRAINT subscription_tenant_id_fkey;
ALTER TABLE public.subscription ADD CONSTRAINT subscription_tenant_id_fkey FOREIGN KEY (tenant_id) REFERENCES tenant(id) ON UPDATE CASCADE ON DELETE CASCADE;
ALTER TABLE public.tenant DROP CONSTRAINT tenant_organization_id_fkey;
ALTER TABLE public.tenant ADD CONSTRAINT tenant_organization_id_fkey FOREIGN KEY (organization_id) REFERENCES organization(id) ON UPDATE CASCADE ON DELETE CASCADE;
ALTER TABLE public.webhook_in_event DROP CONSTRAINT webhook_event_provider_config_id_fkey;
ALTER TABLE public.webhook_in_event ADD CONSTRAINT webhook_event_provider_config_id_fkey FOREIGN KEY (provider_config_id) REFERENCES connector(id) ON UPDATE CASCADE ON DELETE CASCADE;

-- 2. Add the missing tenant_id -> tenant FKs. These columns existed without a
--    constraint, so cascade could not reach them.
ALTER TABLE public.connector               ADD CONSTRAINT connector_tenant_id_fkey               FOREIGN KEY (tenant_id) REFERENCES tenant(id) ON DELETE CASCADE;
ALTER TABLE public.outbox_event            ADD CONSTRAINT outbox_event_tenant_id_fkey            FOREIGN KEY (tenant_id) REFERENCES tenant(id) ON DELETE CASCADE;
ALTER TABLE public.plan_version            ADD CONSTRAINT plan_version_tenant_id_fkey            FOREIGN KEY (tenant_id) REFERENCES tenant(id) ON DELETE CASCADE;
