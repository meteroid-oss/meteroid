-- Comprehensive fixture: one row in every tenant-/org-scoped table, all linked
-- to org O / tenant T / user U. Used to prove a whole-tenant/org delete wipes it.
BEGIN;
-- ids

-- roots
INSERT INTO organization(id,trade_name,slug,default_country) VALUES ('10000000-0000-0000-0000-000000000001','O','o','US');
INSERT INTO "user"(id,email) VALUES ('10000000-0000-0000-0000-000000000003','u@x.com');
INSERT INTO tenant(id,name,slug,organization_id,reporting_currency,environment) VALUES ('10000000-0000-0000-0000-000000000002','T','t','10000000-0000-0000-0000-000000000001','USD','DEVELOPMENT');
-- org-scoped
INSERT INTO role(id,organization_id,name) VALUES ('10000000-0000-0000-0000-000000000022','10000000-0000-0000-0000-000000000001','r');
INSERT INTO organization_member(user_id,organization_id,role,role_id) VALUES ('10000000-0000-0000-0000-000000000003','10000000-0000-0000-0000-000000000001','ADMIN','10000000-0000-0000-0000-000000000022');
INSERT INTO organization_invite(id,organization_id,invited_email,invited_by,expires_at) VALUES (gen_random_uuid(),'10000000-0000-0000-0000-000000000001','i@x.com','10000000-0000-0000-0000-000000000003',now()+interval '1 day');
INSERT INTO org_security_audit_log(id,organization_id,event_type,actor_type) VALUES (gen_random_uuid(),'10000000-0000-0000-0000-000000000001','tenant.deleted','USER');
INSERT INTO connect_oauth_app(id,organization_id,name,client_id,client_secret_hash,client_secret_hint,redirect_uris) VALUES ('10000000-0000-0000-0000-000000000023','10000000-0000-0000-0000-000000000001','app','cid','h','hint','{}');
INSERT INTO connect_authorization_code(id,code_hash,oauth_app_id,connected_organization_id,redirect_uri,scopes,expires_at) VALUES (gen_random_uuid(),'ach','10000000-0000-0000-0000-000000000023','10000000-0000-0000-0000-000000000001','u','{}',now()+interval '1 day');
INSERT INTO connect_access_token(id,token_hash,oauth_app_id,connected_organization_id,scopes,expires_at) VALUES ('10000000-0000-0000-0000-000000000025','th','10000000-0000-0000-0000-000000000023','10000000-0000-0000-0000-000000000001','{}',now()+interval '1 day');
INSERT INTO connect_refresh_token(id,token_hash,access_token_id) VALUES (gen_random_uuid(),'rth','10000000-0000-0000-0000-000000000025');
INSERT INTO connect_connected_account(id,platform_organization_id,connected_organization_id,connection_type,status) VALUES ('10000000-0000-0000-0000-000000000024','10000000-0000-0000-0000-000000000001','10000000-0000-0000-0000-000000000001','STANDARD','ACTIVE');
INSERT INTO connect_platform_settings(organization_id) VALUES ('10000000-0000-0000-0000-000000000001');
INSERT INTO connect_onboarding_link(id,connected_account_id,token_hash,redirect_url,expires_at) VALUES (gen_random_uuid(),'10000000-0000-0000-0000-000000000024','olh','u',now()+interval '1 day');

-- tenant-scoped catalog / core
INSERT INTO invoicing_entity(id,is_default,legal_name,invoice_number_pattern,next_invoice_number,next_credit_note_number,grace_period_hours,net_terms,country,accounting_currency,tenant_id) VALUES ('10000000-0000-0000-0000-000000000004',true,'LN','INV',1,1,0,30,'US','USD','10000000-0000-0000-0000-000000000002');
INSERT INTO custom_tax(id,invoicing_entity_id,tenant_id,name,tax_code) VALUES ('10000000-0000-0000-0000-000000000018','10000000-0000-0000-0000-000000000004','10000000-0000-0000-0000-000000000002','t','TC');
INSERT INTO bank_account(id,tenant_id,currency,country,bank_name,format,account_numbers) VALUES (gen_random_uuid(),'10000000-0000-0000-0000-000000000002','USD','US','B','IBAN_BIC_SWIFT','x');
INSERT INTO connector(id,tenant_id) VALUES ('10000000-0000-0000-0000-000000000012','10000000-0000-0000-0000-000000000002');
INSERT INTO api_token(id,name,tenant_id,hash,hint) VALUES (gen_random_uuid(),'tok','10000000-0000-0000-0000-000000000002','h','hint');
INSERT INTO outbox_event(id,tenant_id,aggregate_id,aggregate_type,event_type) VALUES (gen_random_uuid(),'10000000-0000-0000-0000-000000000002','a','t','e');
INSERT INTO product_family(id,name,tenant_id) VALUES ('10000000-0000-0000-0000-000000000005','PF','10000000-0000-0000-0000-000000000002');
INSERT INTO product(id,name,tenant_id,product_family_id,fee_type,fee_structure) VALUES ('10000000-0000-0000-0000-000000000006','P','10000000-0000-0000-0000-000000000002','10000000-0000-0000-0000-000000000005','RATE','{}');
INSERT INTO product_accounting(product_id,invoicing_entity_id,product_code) VALUES ('10000000-0000-0000-0000-000000000006','10000000-0000-0000-0000-000000000004','pc');
INSERT INTO product_custom_tax(product_id,invoicing_entity_id,custom_tax_id) VALUES ('10000000-0000-0000-0000-000000000006','10000000-0000-0000-0000-000000000004','10000000-0000-0000-0000-000000000018');
INSERT INTO price(id,product_id,cadence,currency,pricing,tenant_id) VALUES ('10000000-0000-0000-0000-00000000000f','10000000-0000-0000-0000-000000000006','MONTHLY','USD','{}','10000000-0000-0000-0000-000000000002');
INSERT INTO billable_metric(id,name,code,tenant_id,product_family_id,product_id) VALUES ('10000000-0000-0000-0000-000000000015','M','c','10000000-0000-0000-0000-000000000002','10000000-0000-0000-0000-000000000005','10000000-0000-0000-0000-000000000006');
INSERT INTO invoicing_entity_email_settings(invoicing_entity_id,tenant_id,sender_name,bcc) VALUES ('10000000-0000-0000-0000-000000000004','10000000-0000-0000-0000-000000000002','S','{b@x.com}');
INSERT INTO email_kind_audience(invoicing_entity_id,tenant_id,kind,mode,audience) VALUES ('10000000-0000-0000-0000-000000000004','10000000-0000-0000-0000-000000000002','invoice_ready','EXCLUDE','{"predicates":[{"attribute":{"key":"country","values":["FR"]}}]}');
INSERT INTO payment_reminder_sequence(id,tenant_id,invoicing_entity_id,name,position,enabled,audience) VALUES ('10000000-0000-0000-0000-000000000026','10000000-0000-0000-0000-000000000002','10000000-0000-0000-0000-000000000004','R',0,true,'{"predicates":[]}');
INSERT INTO payment_reminder_sequence(id,tenant_id,invoicing_entity_id,name,position,enabled) VALUES ('10000000-0000-0000-0000-000000000027','10000000-0000-0000-0000-000000000002','10000000-0000-0000-0000-000000000004','D',1,true);
INSERT INTO payment_reminder_step(id,tenant_id,sequence_id,offset_days,content) VALUES (gen_random_uuid(),'10000000-0000-0000-0000-000000000002','10000000-0000-0000-0000-000000000026',0,'{"en-US":{"subject":"s","body":"b"}}');
INSERT INTO customer(id,name,tenant_id,currency,invoicing_entity_id,payment_reminder_sequence_id) VALUES ('10000000-0000-0000-0000-000000000007','C','10000000-0000-0000-0000-000000000002','USD','10000000-0000-0000-0000-000000000004','10000000-0000-0000-0000-000000000026');
UPDATE connect_connected_account SET platform_customer_id='10000000-0000-0000-0000-000000000007' WHERE id='10000000-0000-0000-0000-000000000024';
UPDATE connect_platform_settings SET branding_invoicing_entity_id='10000000-0000-0000-0000-000000000004' WHERE organization_id='10000000-0000-0000-0000-000000000001';
INSERT INTO customer_connection(id,customer_id,connector_id,external_customer_id) VALUES ('10000000-0000-0000-0000-000000000013','10000000-0000-0000-0000-000000000007','10000000-0000-0000-0000-000000000012','ext');
INSERT INTO customer_payment_method(id,tenant_id,customer_id,connection_id,external_payment_method_id,payment_method_type) VALUES ('10000000-0000-0000-0000-000000000014','10000000-0000-0000-0000-000000000002','10000000-0000-0000-0000-000000000007','10000000-0000-0000-0000-000000000013','epm','CARD');
INSERT INTO coupon(id,code,description,tenant_id,discount,reusable) VALUES ('10000000-0000-0000-0000-00000000000d','CO','d','10000000-0000-0000-0000-000000000002','{}',true);
INSERT INTO plan(id,name,tenant_id,product_family_id,plan_type,status) VALUES ('10000000-0000-0000-0000-000000000008','PL','10000000-0000-0000-0000-000000000002','10000000-0000-0000-0000-000000000005','STANDARD','ACTIVE');
INSERT INTO plan_version(id,is_draft_version,plan_id,tenant_id,net_terms,currency) VALUES ('10000000-0000-0000-0000-000000000009',false,'10000000-0000-0000-0000-000000000008','10000000-0000-0000-0000-000000000002',30,'USD');
INSERT INTO price_component(id,name,plan_version_id,product_id,billable_metric_id) VALUES ('10000000-0000-0000-0000-000000000010','PC','10000000-0000-0000-0000-000000000009','10000000-0000-0000-0000-000000000006','10000000-0000-0000-0000-000000000015');
INSERT INTO plan_component_price(plan_component_id,price_id) VALUES ('10000000-0000-0000-0000-000000000010','10000000-0000-0000-0000-00000000000f');
INSERT INTO coupon_plan(coupon_id,plan_id) VALUES ('10000000-0000-0000-0000-00000000000d','10000000-0000-0000-0000-000000000008');
INSERT INTO add_on(id,name,tenant_id,product_id,price_id) VALUES ('10000000-0000-0000-0000-000000000011','AO','10000000-0000-0000-0000-000000000002','10000000-0000-0000-0000-000000000006','10000000-0000-0000-0000-00000000000f');
INSERT INTO plan_version_add_on(id,plan_version_id,add_on_id,price_id,tenant_id) VALUES (gen_random_uuid(),'10000000-0000-0000-0000-000000000009','10000000-0000-0000-0000-000000000011','10000000-0000-0000-0000-00000000000f','10000000-0000-0000-0000-000000000002');
INSERT INTO invoice_annex_template(id,tenant_id,name,kind,definition) VALUES ('10000000-0000-0000-0000-00000000001c','10000000-0000-0000-0000-000000000002','A','STATIC_TEXT','{}');
INSERT INTO plan_version_annex_template(plan_version_id,annex_template_id,tenant_id) VALUES ('10000000-0000-0000-0000-000000000009','10000000-0000-0000-0000-00000000001c','10000000-0000-0000-0000-000000000002');
INSERT INTO plan_version_adjustment(plan_version_id,kind,amount_subunit) VALUES ('10000000-0000-0000-0000-000000000009','MINIMUM',1);
INSERT INTO plan_version_adjustment_component(plan_version_id,kind,product_id) VALUES ('10000000-0000-0000-0000-000000000009','MINIMUM','10000000-0000-0000-0000-000000000006');
INSERT INTO schedule(id,billing_period,plan_version_id,ramps) VALUES (gen_random_uuid(),'MONTHLY','10000000-0000-0000-0000-000000000009','{}');

-- features/entitlements
INSERT INTO feature(id,tenant_id,product_id,name,code,metric_id) VALUES ('10000000-0000-0000-0000-000000000016','10000000-0000-0000-0000-000000000002','10000000-0000-0000-0000-000000000006','F','fc','10000000-0000-0000-0000-000000000015');
INSERT INTO entitlement(id,tenant_id,feature_id,entity_id,entity_type,mode,value) VALUES (gen_random_uuid(),'10000000-0000-0000-0000-000000000002','10000000-0000-0000-0000-000000000016','10000000-0000-0000-0000-000000000007','CUSTOMER','OVERRIDE','{}');

-- subscription + children
INSERT INTO subscription(id,customer_id,billing_day_anchor,tenant_id,start_date,plan_version_id,net_terms,period,status,current_period_start) VALUES ('10000000-0000-0000-0000-00000000000a','10000000-0000-0000-0000-000000000007',1,'10000000-0000-0000-0000-000000000002',CURRENT_DATE,'10000000-0000-0000-0000-000000000009',30,'MONTHLY','PENDING_ACTIVATION',CURRENT_DATE);
INSERT INTO subscription_component(id,name,subscription_id,price_component_id,product_id,price_id,period) VALUES (gen_random_uuid(),'sc','10000000-0000-0000-0000-00000000000a','10000000-0000-0000-0000-000000000010','10000000-0000-0000-0000-000000000006','10000000-0000-0000-0000-00000000000f','MONTHLY');
INSERT INTO subscription_add_on(id,name,subscription_id,add_on_id,product_id,price_id,period) VALUES (gen_random_uuid(),'sa','10000000-0000-0000-0000-00000000000a','10000000-0000-0000-0000-000000000011','10000000-0000-0000-0000-000000000006','10000000-0000-0000-0000-00000000000f','MONTHLY');
INSERT INTO subscription_adjustment(subscription_id,kind,amount_subunit,includes_all_components) VALUES ('10000000-0000-0000-0000-00000000000a','MINIMUM',1,true);
INSERT INTO subscription_adjustment_component(subscription_id,kind,product_id) VALUES ('10000000-0000-0000-0000-00000000000a','MINIMUM','10000000-0000-0000-0000-000000000006');
INSERT INTO scheduled_event(id,subscription_id,tenant_id,event_type,scheduled_time,priority,event_data,status,source) VALUES (gen_random_uuid(),'10000000-0000-0000-0000-00000000000a','10000000-0000-0000-0000-000000000002','RETRY_PAYMENT',now(),0,'{}','PENDING','x');
INSERT INTO slot_transaction(id,subscription_id,delta,prev_active_slots,effective_at) VALUES (gen_random_uuid(),'10000000-0000-0000-0000-00000000000a',1,0,now());

-- invoice + financial
INSERT INTO invoice(id,tenant_id,customer_id,currency,line_items,invoice_date,billing_period_start,total,net_terms,invoice_number,tax_amount,subtotal_recurring,customer_details,amount_due,subtotal,seller_details,invoicing_entity_id,plan_version_id,subscription_id) VALUES ('10000000-0000-0000-0000-00000000000b','10000000-0000-0000-0000-000000000002','10000000-0000-0000-0000-000000000007','USD','[]',CURRENT_DATE,CURRENT_DATE,0,30,'INV-1',0,0,'{}',0,0,'{}','10000000-0000-0000-0000-000000000004','10000000-0000-0000-0000-000000000009','10000000-0000-0000-0000-00000000000a');
INSERT INTO invoice_payment_reminder(invoice_id,tenant_id,state,resolved_at,generation,sequence_id,enrolled_via,enrolled_at,next_reminder_at) VALUES ('10000000-0000-0000-0000-00000000000b','10000000-0000-0000-0000-000000000002','ENROLLED',now(),1,'10000000-0000-0000-0000-000000000026','CUSTOMER_OVERRIDE',now(),now());
INSERT INTO credit_note(id,credit_note_number,status,tenant_id,customer_id,invoice_id,invoice_number,currency,subtotal,tax_amount,total,customer_details,seller_details,invoicing_entity_id,credit_type,plan_version_id,subscription_id) VALUES ('10000000-0000-0000-0000-00000000000c','CN-1','FINALIZED','10000000-0000-0000-0000-000000000002','10000000-0000-0000-0000-000000000007','10000000-0000-0000-0000-00000000000b','INV-1','USD',0,0,0,'{}','{}','10000000-0000-0000-0000-000000000004','REFUND','10000000-0000-0000-0000-000000000009','10000000-0000-0000-0000-00000000000a');
INSERT INTO payment_transaction(id,tenant_id,invoice_id,amount,currency,status,payment_type,payment_method_id) VALUES (gen_random_uuid(),'10000000-0000-0000-0000-000000000002','10000000-0000-0000-0000-00000000000b',0,'USD','SETTLED','PAYMENT','10000000-0000-0000-0000-000000000014');
-- The shipped balance tables are frozen after the credit ledger upgrade; seed them as the previous release left them.
ALTER TABLE customer_balance_tx DISABLE TRIGGER customer_balance_tx_frozen;
ALTER TABLE customer_balance_pending_tx DISABLE TRIGGER customer_balance_pending_tx_frozen;
INSERT INTO customer_balance_tx(id,amount_cents,balance_cents_after,currency,tenant_id,customer_id,created_by,invoice_id) VALUES ('10000000-0000-0000-0000-000000000017',0,0,'USD','10000000-0000-0000-0000-000000000002','10000000-0000-0000-0000-000000000007','10000000-0000-0000-0000-000000000003','10000000-0000-0000-0000-00000000000b');
INSERT INTO customer_balance_pending_tx(id,amount_cents,invoice_id,tenant_id,customer_id,tx_id,created_by) VALUES (gen_random_uuid(),1,'10000000-0000-0000-0000-00000000000b','10000000-0000-0000-0000-000000000002','10000000-0000-0000-0000-000000000007','10000000-0000-0000-0000-000000000017','10000000-0000-0000-0000-000000000003');
ALTER TABLE customer_balance_tx ENABLE TRIGGER customer_balance_tx_frozen;
ALTER TABLE customer_balance_pending_tx ENABLE TRIGGER customer_balance_pending_tx_frozen;
INSERT INTO customer_credit_purchase(id,amount_cents,invoice_id,tenant_id,customer_id,created_by) VALUES (gen_random_uuid(),1,'10000000-0000-0000-0000-00000000000b','10000000-0000-0000-0000-000000000002','10000000-0000-0000-0000-000000000007','10000000-0000-0000-0000-000000000003');
-- customer credit ledger
INSERT INTO customer_credit_account(id,tenant_id,customer_id,currency) VALUES ('10000000-0000-0000-0000-000000000030','10000000-0000-0000-0000-000000000002','10000000-0000-0000-0000-000000000007','USD');
INSERT INTO customer_credit_grant(id,tenant_id,customer_id,account_id,amount,remaining,priority,classification,effective_at,source_kind,source_id,invoice_id,reason,subscription_id,application) VALUES ('10000000-0000-0000-0000-000000000031','10000000-0000-0000-0000-000000000002','10000000-0000-0000-0000-000000000007','10000000-0000-0000-0000-000000000030',100,90,0,'PAID',now(),'TOP_UP',gen_random_uuid(),'10000000-0000-0000-0000-00000000000b','r','10000000-0000-0000-0000-00000000000a','USAGE');
INSERT INTO customer_credit_entry(id,tenant_id,customer_id,grant_id,kind,amount,source_id,source_kind) VALUES (gen_random_uuid(),'10000000-0000-0000-0000-000000000002','10000000-0000-0000-0000-000000000007','10000000-0000-0000-0000-000000000031','FUND',100,gen_random_uuid(),NULL),(gen_random_uuid(),'10000000-0000-0000-0000-000000000002','10000000-0000-0000-0000-000000000007','10000000-0000-0000-0000-000000000031','DRAW',-10,'10000000-0000-0000-0000-00000000000b','INVOICE');
INSERT INTO customer_credit_operation(id,tenant_id,customer_id,account_id,namespace,idempotency_key,request_fingerprint,amount_minor,invoice_id,reason) VALUES (gen_random_uuid(),'10000000-0000-0000-0000-000000000002','10000000-0000-0000-0000-000000000007','10000000-0000-0000-0000-000000000030','api','k',decode(repeat('00',32),'hex'),100,'10000000-0000-0000-0000-00000000000b','r');
INSERT INTO customer_credit_auto_top_up(account_id,tenant_id,method,threshold_minor,amount_minor) VALUES ('10000000-0000-0000-0000-000000000030','10000000-0000-0000-0000-000000000002','FIXED',10,100);
INSERT INTO api_idempotency_key(tenant_id,operation,idempotency_key,request_fingerprint) VALUES ('10000000-0000-0000-0000-000000000002','op','k',decode(repeat('00',32),'hex'));
INSERT INTO bi_mrr_movement_log(id,description,movement_type,net_mrr_change,currency,applies_to,invoice_id,plan_version_id,tenant_id,credit_note_id,subscription_id,customer_id) VALUES ('10000000-0000-0000-0000-000000000020','d','NEW_BUSINESS',0,'USD',CURRENT_DATE,'10000000-0000-0000-0000-00000000000b','10000000-0000-0000-0000-000000000009','10000000-0000-0000-0000-000000000002','10000000-0000-0000-0000-00000000000c','10000000-0000-0000-0000-00000000000a','10000000-0000-0000-0000-000000000007');
INSERT INTO subscription_event(id,event_type,applies_to,subscription_id,bi_mrr_movement_log_id) VALUES (gen_random_uuid(),'CREATED',CURRENT_DATE,'10000000-0000-0000-0000-00000000000a','10000000-0000-0000-0000-000000000020');
INSERT INTO applied_coupon(id,coupon_id,customer_id,subscription_id,is_active) VALUES (gen_random_uuid(),'10000000-0000-0000-0000-00000000000d','10000000-0000-0000-0000-000000000007','10000000-0000-0000-0000-00000000000a',true);
INSERT INTO checkout_session(id,tenant_id,customer_id,plan_version_id,subscription_id,status) VALUES ('10000000-0000-0000-0000-000000000021','10000000-0000-0000-0000-000000000002','10000000-0000-0000-0000-000000000007','10000000-0000-0000-0000-000000000009','10000000-0000-0000-0000-00000000000a','CREATED');

-- quote + children
INSERT INTO quote(id,tenant_id,customer_id,plan_version_id,currency,quote_number) VALUES ('10000000-0000-0000-0000-00000000000e','10000000-0000-0000-0000-000000000002','10000000-0000-0000-0000-000000000007','10000000-0000-0000-0000-000000000009','USD','Q-1');
INSERT INTO quote_signature(id,quote_id,signed_by_name,signed_by_email) VALUES (gen_random_uuid(),'10000000-0000-0000-0000-00000000000e','n','s@x.com');
INSERT INTO quote_component(id,name,quote_id,price_component_id,product_id,price_id,period) VALUES (gen_random_uuid(),'qc','10000000-0000-0000-0000-00000000000e','10000000-0000-0000-0000-000000000010','10000000-0000-0000-0000-000000000006','10000000-0000-0000-0000-00000000000f','MONTHLY');
INSERT INTO quote_add_on(id,name,quote_id,add_on_id,product_id,price_id,period) VALUES (gen_random_uuid(),'qa','10000000-0000-0000-0000-00000000000e','10000000-0000-0000-0000-000000000011','10000000-0000-0000-0000-000000000006','10000000-0000-0000-0000-00000000000f','MONTHLY');
INSERT INTO quote_coupon(id,quote_id,coupon_id) VALUES (gen_random_uuid(),'10000000-0000-0000-0000-00000000000e','10000000-0000-0000-0000-00000000000d');
INSERT INTO quote_adjustment(quote_id,kind,amount_subunit) VALUES ('10000000-0000-0000-0000-00000000000e','MINIMUM',1);
INSERT INTO quote_adjustment_component(quote_id,kind,product_id) VALUES ('10000000-0000-0000-0000-00000000000e','MINIMUM','10000000-0000-0000-0000-000000000006');

-- webhooks / connectors
INSERT INTO webhook_in_event(id,key,provider_config_id) VALUES (gen_random_uuid(),'k','10000000-0000-0000-0000-000000000012');

-- analytics / alerts / notifications
INSERT INTO alert_definition(id,tenant_id,name,metric_key,condition_kind,condition) VALUES ('10000000-0000-0000-0000-00000000001b','10000000-0000-0000-0000-000000000002','A','m','THRESHOLD','{}');
INSERT INTO alert_event(id,tenant_id,alert_definition_id,window_date,metric_value,severity,customer_id) VALUES (gen_random_uuid(),'10000000-0000-0000-0000-000000000002','10000000-0000-0000-0000-00000000001b',CURRENT_DATE,1,'WARNING','10000000-0000-0000-0000-000000000007');
INSERT INTO analytics_saved_view(id,tenant_id,name,metric_key,visualization,config) VALUES ('10000000-0000-0000-0000-00000000001d','10000000-0000-0000-0000-000000000002','V','m','v','{}');
INSERT INTO analytics_dashboard_share(id,tenant_id,saved_view_id,token,snapshot) VALUES (gen_random_uuid(),'10000000-0000-0000-0000-000000000002','10000000-0000-0000-0000-00000000001d','tk','{}');
INSERT INTO segment(id,tenant_id,name,definition) VALUES ('10000000-0000-0000-0000-00000000001f','10000000-0000-0000-0000-000000000002','S','{}');
INSERT INTO alert_segment_member(alert_definition_id,tenant_id,customer_id) VALUES ('10000000-0000-0000-0000-00000000001b','10000000-0000-0000-0000-000000000002','10000000-0000-0000-0000-000000000007');
INSERT INTO notification_channel(id,tenant_id,name,channel_type,config) VALUES (gen_random_uuid(),'10000000-0000-0000-0000-000000000002','N','SLACK','{}');

-- activity / email / batch / dlq / stored objects / sandbox pool
INSERT INTO entity_activity(id,tenant_id,entity_type,entity_id,activity_type,actor_type,actor_uuid) VALUES ('10000000-0000-0000-0000-00000000001e','10000000-0000-0000-0000-000000000002','customer','10000000-0000-0000-0000-000000000007','x','USER','10000000-0000-0000-0000-000000000003');
INSERT INTO sent_email(id,tenant_id,subject,from_addr,recipients,body_html,kind,locale,invoicing_entity_id,context,dedupe_key) VALUES ('10000000-0000-0000-0000-00000000001e','10000000-0000-0000-0000-000000000002','s','f@x.com','{r@x.com}','<p></p>','invoice_ready','en-US','10000000-0000-0000-0000-000000000004','{"type":"invoice","invoice_id":"10000000-0000-0000-0000-00000000000b"}','invoice_ready:10000000-0000-0000-0000-00000000000b');
INSERT INTO entity_activity(id,tenant_id,entity_type,entity_id,activity_type,actor_type) VALUES ('10000000-0000-0000-0000-000000000028','10000000-0000-0000-0000-000000000002','invoice','10000000-0000-0000-0000-00000000000b','entity.email_resent','SYSTEM');
INSERT INTO sent_email(id,tenant_id,subject,from_addr,recipients,body_html,kind,resent_from) VALUES ('10000000-0000-0000-0000-000000000028','10000000-0000-0000-0000-000000000002','s','f@x.com','{r@x.com}','<p></p>','invoice_ready','10000000-0000-0000-0000-00000000001e');
INSERT INTO email_send_claim(dedupe_key,tenant_id,state,claimed_at,finished_at,sent_email_id) VALUES ('invoice_ready:10000000-0000-0000-0000-00000000000b','10000000-0000-0000-0000-000000000002','SENT',now(),now(),'10000000-0000-0000-0000-00000000001e');
INSERT INTO batch_job(id,tenant_id,job_type,created_by) VALUES ('10000000-0000-0000-0000-000000000019','10000000-0000-0000-0000-000000000002','EVENT_CSV_IMPORT','10000000-0000-0000-0000-000000000003');
INSERT INTO batch_job_chunk(id,job_id,tenant_id,chunk_index,item_offset,item_count) VALUES ('10000000-0000-0000-0000-00000000001a','10000000-0000-0000-0000-000000000019','10000000-0000-0000-0000-000000000002',0,0,1);
INSERT INTO batch_job_item_failure(id,chunk_id,job_id,tenant_id,item_index,reason) VALUES (gen_random_uuid(),'10000000-0000-0000-0000-00000000001a','10000000-0000-0000-0000-000000000019','10000000-0000-0000-0000-000000000002',0,'r');
INSERT INTO batch_job_entity(id,batch_job_id,tenant_id,entity_type,entity_id) VALUES (gen_random_uuid(),'10000000-0000-0000-0000-000000000019','10000000-0000-0000-0000-000000000002','customer','10000000-0000-0000-0000-000000000007');
INSERT INTO dead_letter_message(id,tenant_id,queue,pgmq_msg_id,read_ct,enqueued_at) VALUES (gen_random_uuid(),'10000000-0000-0000-0000-000000000002','q',1,0,now());
INSERT INTO stored_object(id,tenant_id,kind,key) VALUES (gen_random_uuid(),'10000000-0000-0000-0000-000000000002','INVOICE_PDF','k');
INSERT INTO sandbox_pool_tenant(tenant_id) VALUES ('10000000-0000-0000-0000-000000000002');
-- global (not tenant/org-scoped) tables; must SURVIVE the delete
INSERT INTO historical_rates_from_usd(id,date,rates) VALUES ('10000000-0000-0000-0000-0000000000f0','1970-01-02','{}');
INSERT INTO oauth_verifier(id,csrf_token,pkce_verifier) VALUES ('10000000-0000-0000-0000-0000000000f1','fixture-csrf','fixture-pkce');
COMMIT;
