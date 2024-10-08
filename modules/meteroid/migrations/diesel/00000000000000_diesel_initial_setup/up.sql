create type "BillingMetricAggregateEnum" as enum ('COUNT', 'LATEST', 'MAX', 'MIN', 'MEAN', 'SUM', 'COUNT_DISTINCT');

create type "UnitConversionRoundingEnum" as enum ('UP', 'DOWN', 'NEAREST', 'NEAREST_HALF', 'NEAREST_DECILE', 'NONE');

create type "OrganizationUserRole" as enum ('ADMIN', 'MEMBER');

create type "PlanTypeEnum" as enum ('STANDARD', 'FREE', 'CUSTOM');

create type "PlanStatusEnum" as enum ('DRAFT', 'ACTIVE', 'INACTIVE', 'ARCHIVED');

create type "BillingPeriodEnum" as enum ('MONTHLY', 'QUARTERLY', 'ANNUAL');

create type "InvoiceExternalStatusEnum" as enum ('DELETED', 'DRAFT', 'FINALIZED', 'PAID', 'PAYMENT_FAILED', 'UNCOLLECTIBLE', 'VOID');

create type "InvoicingProviderEnum" as enum ('STRIPE', 'MANUAL');

create type "InvoiceStatusEnum" as enum ('DRAFT', 'FINALIZED', 'PENDING', 'VOID');

create type fang_task_state as enum ('new', 'in_progress', 'failed', 'finished', 'retried');

create type "WebhookOutEventTypeEnum" as enum ('CUSTOMER_CREATED', 'SUBSCRIPTION_CREATED', 'INVOICE_CREATED', 'INVOICE_FINALIZED');

create type "InvoiceType" as enum ('RECURRING', 'ONE_OFF', 'ADJUSTMENT', 'IMPORTED', 'USAGE_THRESHOLD');

create type "CreditNoteStatus" as enum ('DRAFT', 'FINALIZED', 'VOIDED');

create type "MRRMovementType" as enum ('NEW_BUSINESS', 'EXPANSION', 'CONTRACTION', 'CHURN', 'REACTIVATION');

create type "SubscriptionFeeBillingPeriod" as enum ('ONE_TIME', 'MONTHLY', 'QUARTERLY', 'ANNUAL');

create type "TenantEnvironmentEnum" as enum ('PRODUCTION', 'STAGING', 'QA', 'DEVELOPMENT', 'SANDBOX', 'DEMO');

create type "SubscriptionEventType" as enum ('CREATED', 'ACTIVATED', 'SWITCH', 'CANCELLED', 'REACTIVATED', 'UPDATED');

create table if not exists organization
(
  id               uuid                                   not null
    primary key,
  trade_name       text                                   not null,
  slug             text                                   not null,
  created_at       timestamp(3) default CURRENT_TIMESTAMP not null,
  archived_at      timestamp(3),
  invite_link_hash text,
  default_country  text                                   not null
);

create unique index if not exists organization_slug_key
  on organization (slug);

create unique index if not exists organization_invite_link_hash_idx
  on organization (invite_link_hash);

create table if not exists "user"
(
  id            uuid                                   not null
    primary key,
  email         text                                   not null,
  created_at    timestamp(3) default CURRENT_TIMESTAMP not null,
  archived_at   timestamp(3),
  password_hash text,
  onboarded     boolean      default false             not null,
  first_name    text,
  last_name     text,
  department    text
);

create unique index if not exists user_email_key
  on "user" (email);

create table if not exists tenant
(
  id              uuid                                                                   not null
    primary key,
  name            text                                                                   not null,
  slug            text                                                                   not null,
  created_at      timestamp(3)            default CURRENT_TIMESTAMP                      not null,
  updated_at      timestamp(3),
  archived_at     timestamp(3),
  organization_id uuid                                                                   not null
    references organization
      on update cascade on delete restrict,
  currency        text                                                                   not null,
  environment     "TenantEnvironmentEnum" default 'DEVELOPMENT'::"TenantEnvironmentEnum" not null
);

create unique index if not exists tenant_slug_key
  on tenant (slug);

create unique index if not exists tenant_name_organization_id_key
  on tenant (name, organization_id);

create table if not exists product_family
(
  id          uuid                                   not null
    primary key,
  name        text                                   not null,
  external_id text                                   not null,
  created_at  timestamp(3) default CURRENT_TIMESTAMP not null,
  updated_at  timestamp(3),
  archived_at timestamp(3),
  tenant_id   uuid                                   not null
    references tenant
      on update cascade on delete restrict
);

create unique index if not exists product_family_api_name_tenant_id_key
  on product_family (external_id, tenant_id);

create unique index if not exists product_family_external_id_tenant_id_key
  on product_family (external_id, tenant_id);

create table if not exists billable_metric
(
  id                       uuid                                                                       not null
    primary key,
  name                     text                                                                       not null,
  description              text,
  code                     text                                                                       not null,
  aggregation_type         "BillingMetricAggregateEnum" default 'COUNT'::"BillingMetricAggregateEnum" not null,
  aggregation_key          text,
  unit_conversion_factor   integer,
  unit_conversion_rounding "UnitConversionRoundingEnum" default 'NONE'::"UnitConversionRoundingEnum",
  segmentation_matrix      jsonb,
  usage_group_key          text,
  created_at               timestamp(3)                 default CURRENT_TIMESTAMP                     not null,
  created_by               uuid                                                                       not null,
  updated_at               timestamp(3),
  archived_at              timestamp(3),
  tenant_id                uuid                                                                       not null
    references tenant
      on update cascade on delete restrict,
  product_family_id        uuid                                                                       not null
    references product_family
      on update cascade on delete restrict
);

create table if not exists plan
(
  id                uuid                                   not null
    primary key,
  name              text                                   not null,
  description       text,
  created_at        timestamp(3) default CURRENT_TIMESTAMP not null,
  created_by        uuid                                   not null,
  updated_at        timestamp(3),
  archived_at       timestamp(3),
  tenant_id         uuid                                   not null
    references tenant
      on update cascade on delete restrict,
  product_family_id uuid                                   not null
    references product_family
      on update cascade on delete restrict,
  external_id       text                                   not null,
  plan_type         "PlanTypeEnum"                         not null,
  status            "PlanStatusEnum"                       not null
);

create unique index if not exists plan_tenant_id_external_id_key
  on plan (tenant_id, external_id);

create table if not exists product
(
  id                uuid                                   not null
    primary key,
  name              text                                   not null,
  description       text,
  created_at        timestamp(3) default CURRENT_TIMESTAMP not null,
  created_by        uuid                                   not null,
  updated_at        timestamp(3),
  archived_at       timestamp(3),
  tenant_id         uuid                                   not null
    references tenant
      on update cascade on delete restrict,
  product_family_id uuid                                   not null
    references product_family
      on update cascade on delete set null
);

create table if not exists api_token
(
  id         uuid                                   not null
    primary key,
  name       text                                   not null,
  created_at timestamp(3) default CURRENT_TIMESTAMP not null,
  created_by uuid                                   not null,
  tenant_id  uuid                                   not null
    references tenant
      on update cascade on delete restrict,
  hash       text                                   not null,
  hint       text                                   not null
);

create unique index if not exists api_token_hash_key
  on api_token (hash);

create table if not exists organization_member
(
  user_id         uuid                   not null
    references "user"
      on update cascade on delete restrict,
  organization_id uuid                   not null
    references organization
      on update cascade on delete restrict,
  role            "OrganizationUserRole" not null,
  primary key (user_id, organization_id)
);

create table if not exists plan_version
(
  id                     uuid                                   not null
    primary key,
  is_draft_version       boolean                                not null,
  plan_id                uuid                                   not null
    references plan
      on update cascade on delete restrict,
  version                integer      default 1                 not null,
  trial_duration_days    integer,
  trial_fallback_plan_id uuid,
  tenant_id              uuid                                   not null,
  period_start_day       smallint,
  net_terms              integer                                not null,
  currency               text                                   not null,
  billing_cycles         integer,
  created_at             timestamp(3) default CURRENT_TIMESTAMP not null,
  created_by             uuid                                   not null,
  billing_periods        "BillingPeriodEnum"[]                  not null,
  constraint plan_version_check
    check (((trial_duration_days IS NULL) AND (trial_fallback_plan_id IS NULL)) OR
           ((trial_duration_days IS NOT NULL) AND (trial_fallback_plan_id IS NOT NULL)))
);

create index if not exists idx_plan_version
  on plan_version (plan_id asc, version desc);

create table if not exists price_component
(
  id                 uuid  not null
    primary key,
  name               text  not null,
  fee                jsonb not null,
  plan_version_id    uuid  not null
    references plan_version
      on update cascade on delete cascade,
  product_item_id    uuid
                           references product
                             on update cascade on delete set null,
  billable_metric_id uuid
                           references billable_metric
                             on update cascade on delete set null
);

create table if not exists schedule
(
  id              uuid                not null
    primary key,
  billing_period  "BillingPeriodEnum" not null,
  plan_version_id uuid                not null
    references plan_version
      on update cascade on delete cascade,
  ramps           jsonb               not null
);

create table if not exists provider_config
(
  id                 uuid                                   not null
    primary key,
  created_at         timestamp(3) default CURRENT_TIMESTAMP not null,
  tenant_id          uuid                                   not null,
  invoicing_provider "InvoicingProviderEnum"                not null,
  enabled            boolean      default true              not null,
  webhook_security   jsonb                                  not null,
  api_security       jsonb                                  not null
);

create table if not exists webhook_in_event
(
  id                 uuid                                                  not null
    constraint webhook_event_pkey
      primary key,
  received_at        timestamp(3) with time zone default CURRENT_TIMESTAMP not null,
  action             text,
  key                text                                                  not null,
  processed          boolean                     default false             not null,
  attempts           integer                     default 0                 not null,
  error              text,
  provider_config_id uuid                                                  not null
    constraint webhook_event_provider_config_id_fkey
      references provider_config
      on update cascade on delete restrict
);

create index if not exists provider_config_tenant_id
  on provider_config (tenant_id);

create unique index if not exists provider_config_uniqueness_idx
  on provider_config (tenant_id, invoicing_provider)
  where (enabled = true);

create table if not exists fang_tasks
(
  id            uuid                     default gen_random_uuid()           not null
    primary key,
  metadata      jsonb                                                        not null,
  error_message text,
  state         fang_task_state          default 'new'::fang_task_state      not null,
  task_type     varchar                  default 'common'::character varying not null,
  uniq_hash     char(64),
  retries       integer                  default 0                           not null,
  scheduled_at  timestamp with time zone default now()                       not null,
  created_at    timestamp with time zone default now()                       not null,
  updated_at    timestamp with time zone default now()                       not null
);

create index if not exists fang_tasks_state_index
  on fang_tasks (state);

create index if not exists fang_tasks_type_index
  on fang_tasks (task_type);

create index if not exists fang_tasks_scheduled_at_index
  on fang_tasks (scheduled_at);

create index if not exists fang_tasks_uniq_hash
  on fang_tasks (uniq_hash);

create table if not exists fang_tasks_archive
(
  id            uuid                     default gen_random_uuid()           not null
    primary key,
  metadata      jsonb                                                        not null,
  error_message text,
  state         fang_task_state          default 'new'::fang_task_state      not null,
  task_type     varchar                  default 'common'::character varying not null,
  uniq_hash     char(64),
  retries       integer                  default 0                           not null,
  scheduled_at  timestamp with time zone default now()                       not null,
  created_at    timestamp with time zone default now()                       not null,
  updated_at    timestamp with time zone default now()                       not null,
  archived_at   timestamp with time zone default now()                       not null
);

create index if not exists fang_tasks_archive_archived_at_index
  on fang_tasks_archive (archived_at);

create table if not exists webhook_out_endpoint
(
  id               uuid                                   not null
    primary key,
  tenant_id        uuid                                   not null
    references tenant
      on update cascade on delete restrict,
  url              text                                   not null,
  description      text,
  secret           text                                   not null,
  created_at       timestamp(3) default CURRENT_TIMESTAMP not null,
  events_to_listen "WebhookOutEventTypeEnum"[]            not null,
  enabled          boolean                                not null
);

create index if not exists webhook_out_endpoint_tenant_id_idx
  on webhook_out_endpoint (tenant_id);

create table if not exists webhook_out_event
(
  id               uuid                                   not null
    primary key,
  endpoint_id      uuid                                   not null
    references webhook_out_endpoint
      on update cascade on delete restrict,
  created_at       timestamp(3) default CURRENT_TIMESTAMP not null,
  event_type       "WebhookOutEventTypeEnum"              not null,
  request_body     text                                   not null,
  response_body    text,
  http_status_code smallint,
  error_message    text
);

create index if not exists webhook_out_event_endpoint_id_timestamp_idx
  on webhook_out_event (endpoint_id asc, created_at desc);

create table if not exists bi_customer_ytd_summary
(
  tenant_id           uuid    not null,
  customer_id         uuid    not null,
  revenue_year        integer not null,
  currency            text    not null,
  total_revenue_cents bigint  not null,
  primary key (tenant_id, customer_id, currency, revenue_year)
);

create table if not exists historical_rates_from_usd
(
  id    uuid  not null
    primary key,
  date  date  not null
    unique,
  rates jsonb not null
);

create table if not exists bi_delta_mrr_daily
(
  tenant_id              uuid    not null,
  plan_version_id        uuid    not null,
  date                   date    not null,
  currency               text    not null,
  net_mrr_cents          bigint  not null,
  new_business_cents     bigint  not null,
  new_business_count     integer not null,
  expansion_cents        bigint  not null,
  expansion_count        integer not null,
  contraction_cents      bigint  not null,
  contraction_count      integer not null,
  churn_cents            bigint  not null,
  churn_count            integer not null,
  reactivation_cents     bigint  not null,
  reactivation_count     integer not null,
  historical_rate_id     uuid    not null
    references historical_rates_from_usd
      on update cascade on delete restrict,
  net_mrr_cents_usd      bigint  not null,
  new_business_cents_usd bigint  not null,
  expansion_cents_usd    bigint  not null,
  contraction_cents_usd  bigint  not null,
  churn_cents_usd        bigint  not null,
  reactivation_cents_usd bigint  not null,
  primary key (tenant_id, plan_version_id, currency, date)
);

create table if not exists bi_revenue_daily
(
  tenant_id             uuid                           not null,
  plan_version_id       uuid,
  currency              text                           not null,
  revenue_date          date                           not null,
  net_revenue_cents     bigint                         not null,
  historical_rate_id    uuid                           not null
    references historical_rates_from_usd
      on update cascade on delete restrict,
  net_revenue_cents_usd bigint                         not null,
  id                    uuid default gen_random_uuid() not null
    primary key
);

create unique index if not exists bi_revenue_daily_uniqueness
  on bi_revenue_daily (tenant_id, plan_version_id, currency, revenue_date);

create table if not exists add_on
(
  id         uuid                                   not null
    primary key,
  name       text                                   not null,
  fee        jsonb                                  not null,
  tenant_id  uuid                                   not null
    references tenant
      on update cascade on delete restrict,
  created_at timestamp(3) default CURRENT_TIMESTAMP not null,
  updated_at timestamp(3) default CURRENT_TIMESTAMP not null
);

create index if not exists add_on_tenant_id_idx
  on add_on (tenant_id);

create table if not exists invoicing_entity
(
  id                      uuid        not null
    primary key,
  local_id                text        not null,
  is_default              boolean     not null,
  legal_name              text        not null,
  invoice_number_pattern  text        not null,
  next_invoice_number     bigint      not null,
  next_credit_note_number bigint      not null,
  grace_period_hours      integer     not null,
  net_terms               integer     not null,
  invoice_footer_info     text,
  invoice_footer_legal    text,
  logo_attachment_id      text,
  brand_color             text,
  address_line1           text,
  address_line2           text,
  zip_code                varchar(50),
  state                   text,
  city                    text,
  vat_number              text,
  country                 text        not null,
  accounting_currency     varchar(50) not null,
  tenant_id               uuid        not null
    references tenant
      on update cascade on delete cascade,
  unique (local_id, tenant_id)
);

create table if not exists customer
(
  id                  uuid                                   not null
    primary key,
  name                text                                   not null,
  created_at          timestamp(3) default CURRENT_TIMESTAMP not null,
  created_by          uuid                                   not null,
  updated_at          timestamp(3),
  updated_by          uuid,
  archived_at         timestamp(3),
  tenant_id           uuid                                   not null
    references tenant
      on update cascade on delete restrict,
  billing_config      jsonb                                  not null,
  alias               text,
  email               text,
  invoicing_email     text,
  phone               text,
  balance_value_cents integer      default 0                 not null
    constraint customer_balance_non_negative
      check (balance_value_cents >= 0),
  currency            text                                   not null,
  billing_address     jsonb,
  shipping_address    jsonb,
  invoicing_entity_id uuid                                   not null
    references invoicing_entity
      on delete restrict
);

create unique index if not exists customer_tenant_id_alias_idx
  on customer (tenant_id, alias);

create table if not exists subscription
(
  id                  uuid                                          not null
    primary key,
  customer_id         uuid                                          not null
    references customer
      on update cascade on delete restrict,
  billing_day         smallint                                      not null,
  tenant_id           uuid                                          not null
    references tenant
      on update cascade on delete restrict,
  trial_start_date    date,
  billing_start_date  date                                          not null,
  billing_end_date    date,
  plan_version_id     uuid                                          not null
    references plan_version
      on update cascade on delete restrict,
  created_at          timestamp(3) default CURRENT_TIMESTAMP        not null,
  created_by          uuid                                          not null,
  net_terms           integer                                       not null,
  invoice_memo        text,
  invoice_threshold   numeric,
  activated_at        timestamp(3),
  canceled_at         timestamp(3),
  cancellation_reason text,
  currency            varchar(3)   default 'USD'::character varying not null,
  mrr_cents           bigint       default 0                        not null,
  period              "BillingPeriodEnum"                           not null
);

create index if not exists subscription_billing_day_idx
  on subscription (billing_day);

create index if not exists subscription_start_date_idx
  on subscription (billing_start_date);

create index if not exists subscription_end_date_idx
  on subscription (billing_end_date);

create table if not exists invoice
(
  id                    uuid                                                             not null
    primary key,
  status                "InvoiceStatusEnum"         default 'DRAFT'::"InvoiceStatusEnum" not null,
  external_status       "InvoiceExternalStatusEnum",
  created_at            timestamp(3) with time zone default CURRENT_TIMESTAMP            not null,
  updated_at            timestamp(3) with time zone,
  tenant_id             uuid                                                             not null
    references tenant
      on update cascade on delete restrict,
  customer_id           uuid                                                             not null
    references customer
      on update cascade on delete restrict,
  subscription_id       uuid,
  currency              text                                                             not null,
  external_invoice_id   text,
  invoicing_provider    "InvoicingProviderEnum"                                          not null,
  line_items            jsonb                                                            not null,
  issued                boolean                     default false                        not null,
  issue_attempts        integer                     default 0                            not null,
  last_issue_attempt_at timestamp(3) with time zone,
  last_issue_error      text,
  data_updated_at       timestamp(3),
  invoice_date          date                                                             not null,
  total                 bigint                                                           not null,
  plan_version_id       uuid
    references plan_version
      on update cascade on delete restrict,
  invoice_type          "InvoiceType"               default 'RECURRING'::"InvoiceType"   not null,
  finalized_at          timestamp(3),
  net_terms             integer                                                          not null,
  memo                  text,
  tax_rate              integer                                                          not null,
  local_id              text                                                             not null,
  reference             text,
  invoice_number        text                                                             not null,
  tax_amount            bigint                                                           not null,
  subtotal_recurring    bigint                                                           not null,
  plan_name             text,
  due_at                timestamp(3),
  customer_details      jsonb                                                            not null,
  amount_due            bigint                                                           not null,
  subtotal              bigint                                                           not null,
  applied_credits       bigint                      default 0                            not null,
  seller_details        jsonb                                                            not null
);

create unique index if not exists invoice_external_invoice_id_key
  on invoice (external_invoice_id, tenant_id);

create unique index if not exists invoice_invoice_number_key
  on invoice (invoice_number, tenant_id)
  where (status <> 'DRAFT'::"InvoiceStatusEnum");

create table if not exists slot_transaction
(
  id                 uuid                                   not null
    primary key,
  price_component_id uuid                                   not null
    references price_component
      on update cascade on delete restrict,
  subscription_id    uuid                                   not null
    references subscription
      on update cascade on delete restrict,
  delta              integer                                not null,
  prev_active_slots  integer                                not null,
  effective_at       timestamp(3)                           not null,
  transaction_at     timestamp(3) default CURRENT_TIMESTAMP not null
);

create index if not exists slot_transaction_sub_price_comp_idx
  on slot_transaction (subscription_id, price_component_id);

create table if not exists credit_note
(
  id                    uuid               not null
    primary key,
  created_at            timestamp(3)       not null,
  updated_at            timestamp(3)       not null,
  refunded_amount_cents bigint,
  credited_amount_cents bigint,
  currency              text               not null,
  finalized_at          timestamp(3)       not null,
  plan_version_id       uuid
                                           references plan_version
                                             on update cascade on delete set null,
  invoice_id            uuid               not null
    references invoice
      on update cascade on delete restrict,
  tenant_id             uuid               not null
    references tenant
      on update cascade on delete restrict,
  customer_id           uuid               not null
    references customer
      on update cascade on delete restrict,
  status                "CreditNoteStatus" not null
);

create table if not exists bi_mrr_movement_log
(
  id              uuid                                   not null
    primary key,
  description     text                                   not null,
  movement_type   "MRRMovementType"                      not null,
  net_mrr_change  bigint                                 not null,
  currency        varchar(3)                             not null,
  created_at      timestamp(3) default CURRENT_TIMESTAMP not null,
  applies_to      date                                   not null,
  invoice_id      uuid                                   not null
    references invoice
      on update cascade on delete restrict,
  credit_note_id  uuid
    references credit_note
      on update cascade on delete restrict,
  plan_version_id uuid                                   not null
    references plan_version
      on update cascade on delete restrict,
  tenant_id       uuid                                   not null
    references tenant
      on update cascade on delete restrict
);

create index if not exists bi_mrr_movement_log_idx
  on bi_mrr_movement_log (tenant_id, applies_to);

create table if not exists subscription_component
(
  id                 uuid                           not null
    primary key,
  name               text                           not null,
  subscription_id    uuid                           not null
    references subscription
      on delete cascade,
  price_component_id uuid
    references price_component
      on delete cascade,
  product_item_id    uuid
    references product
      on delete cascade,
  period             "SubscriptionFeeBillingPeriod" not null,
  fee                jsonb                          not null
);

create table if not exists subscription_event
(
  id                     uuid                                   not null
    primary key,
  mrr_delta              bigint,
  event_type             "SubscriptionEventType"                not null,
  created_at             timestamp(3) default CURRENT_TIMESTAMP not null,
  applies_to             date                                   not null,
  subscription_id        uuid                                   not null
    references subscription
      on delete cascade,
  bi_mrr_movement_log_id uuid
    references bi_mrr_movement_log
      on delete cascade,
  details                jsonb
);

create table if not exists customer_balance_tx
(
  id                  uuid                                   not null
    primary key,
  created_at          timestamp(3) default CURRENT_TIMESTAMP not null,
  amount_cents        integer                                not null,
  balance_cents_after integer                                not null,
  note                text,
  invoice_id          uuid
    references invoice
      on update cascade on delete restrict,
  tenant_id           uuid                                   not null
    references tenant
      on update cascade on delete restrict,
  customer_id         uuid                                   not null
    references customer
      on update cascade on delete restrict,
  created_by          uuid
    references "user"
      on update cascade on delete restrict
);

create unique index if not exists customer_balance_tx_invoice_id
  on customer_balance_tx (invoice_id);

create table if not exists customer_balance_pending_tx
(
  id           uuid                                   not null
    primary key,
  created_at   timestamp(3) default CURRENT_TIMESTAMP not null,
  updated_at   timestamp(3) default CURRENT_TIMESTAMP not null,
  amount_cents integer                                not null
    constraint customer_balance_pending_tx_amount_positive
      check (amount_cents > 0),
  note         text,
  invoice_id   uuid                                   not null
    references invoice
      on update cascade on delete restrict,
  tenant_id    uuid                                   not null
    references tenant
      on update cascade on delete restrict,
  customer_id  uuid                                   not null
    references customer
      on update cascade on delete restrict,
  tx_id        uuid
    references customer_balance_tx
      on update cascade on delete restrict,
  created_by   uuid                                   not null
    references "user"
      on update cascade on delete restrict
);

create unique index if not exists customer_balance_pending_tx_invoice_id
  on customer_balance_pending_tx (invoice_id);

create unique index if not exists invoicing_entity_is_default_tenant_id_key
  on invoicing_entity (tenant_id)
  where (is_default = true);

create table if not exists subscription_add_on
(
  id              uuid                                   not null
    primary key,
  name            text                                   not null,
  subscription_id uuid                                   not null
    references subscription
      on delete cascade,
  add_on_id       uuid                                   not null
    references add_on
      on delete cascade,
  period          "SubscriptionFeeBillingPeriod"         not null,
  fee             jsonb                                  not null,
  created_at      timestamp(3) default CURRENT_TIMESTAMP not null
);

create index if not exists subscription_add_on_subscription_id_idx
  on subscription_add_on (subscription_id);

create or replace function fn_update_mrr() returns trigger
  language plpgsql
as
$$
DECLARE
  net_mrr_change_usd BIGINT;
  historical_rate_record RECORD;
BEGIN
  SELECT id, (rates->>NEW.currency)::NUMERIC as rate INTO historical_rate_record
  FROM historical_rates_from_usd
  WHERE date <= NEW.applies_to
  ORDER BY date DESC
  LIMIT 1;

  net_mrr_change_usd := NEW.net_mrr_change / historical_rate_record.rate;

  INSERT INTO bi_delta_mrr_daily (
    tenant_id,
    plan_version_id,
    currency,
    date,
    net_mrr_cents,
    net_mrr_cents_usd,
    new_business_cents,
    new_business_cents_usd,
    new_business_count,
    expansion_cents,
    expansion_cents_usd,
    expansion_count,
    contraction_cents,
    contraction_cents_usd,
    contraction_count,
    churn_cents,
    churn_cents_usd,
    churn_count,
    reactivation_cents,
    reactivation_cents_usd,
    reactivation_count,
    historical_rate_id
  )
  VALUES (
           NEW.tenant_id,
           NEW.plan_version_id,
           NEW.currency,
           NEW.applies_to,
           NEW.net_mrr_change,
           net_mrr_change_usd,
           CASE WHEN NEW.movement_type = 'NEW_BUSINESS' THEN NEW.net_mrr_change ELSE 0 END,
           CASE WHEN NEW.movement_type = 'NEW_BUSINESS' THEN net_mrr_change_usd ELSE 0 END,
           CASE WHEN NEW.movement_type = 'NEW_BUSINESS' THEN 1 ELSE 0 END,
           CASE WHEN NEW.movement_type = 'EXPANSION' THEN NEW.net_mrr_change ELSE 0 END,
           CASE WHEN NEW.movement_type = 'EXPANSION' THEN net_mrr_change_usd ELSE 0 END,
           CASE WHEN NEW.movement_type = 'EXPANSION' THEN 1 ELSE 0 END,
           CASE WHEN NEW.movement_type = 'CONTRACTION' THEN NEW.net_mrr_change ELSE 0 END,
           CASE WHEN NEW.movement_type = 'CONTRACTION' THEN net_mrr_change_usd ELSE 0 END,
           CASE WHEN NEW.movement_type = 'CONTRACTION' THEN 1 ELSE 0 END,
           CASE WHEN NEW.movement_type = 'CHURN' THEN NEW.net_mrr_change ELSE 0 END,
           CASE WHEN NEW.movement_type = 'CHURN' THEN net_mrr_change_usd ELSE 0 END,
           CASE WHEN NEW.movement_type = 'CHURN' THEN 1 ELSE 0 END,
           CASE WHEN NEW.movement_type = 'REACTIVATION' THEN NEW.net_mrr_change ELSE 0 END,
           CASE WHEN NEW.movement_type = 'REACTIVATION' THEN net_mrr_change_usd ELSE 0 END,
           CASE WHEN NEW.movement_type = 'REACTIVATION' THEN 1 ELSE 0 END,
           historical_rate_record.id
         )
  ON CONFLICT (tenant_id, plan_version_id, currency, date) DO UPDATE
    SET
      net_mrr_cents = bi_delta_mrr_daily.net_mrr_cents + EXCLUDED.net_mrr_cents,
      net_mrr_cents_usd = bi_delta_mrr_daily.net_mrr_cents_usd + EXCLUDED.net_mrr_cents_usd,
      new_business_cents = bi_delta_mrr_daily.new_business_cents + EXCLUDED.new_business_cents,
      new_business_cents_usd = bi_delta_mrr_daily.new_business_cents_usd + EXCLUDED.new_business_cents_usd,
      new_business_count = bi_delta_mrr_daily.new_business_count + EXCLUDED.new_business_count,
      expansion_cents = bi_delta_mrr_daily.expansion_cents + EXCLUDED.expansion_cents,
      expansion_cents_usd = bi_delta_mrr_daily.expansion_cents_usd + EXCLUDED.expansion_cents_usd,
      expansion_count = bi_delta_mrr_daily.expansion_count + EXCLUDED.expansion_count,
      contraction_cents = bi_delta_mrr_daily.contraction_cents + EXCLUDED.contraction_cents,
      contraction_cents_usd = bi_delta_mrr_daily.contraction_cents_usd + EXCLUDED.contraction_cents_usd,
      contraction_count = bi_delta_mrr_daily.contraction_count + EXCLUDED.contraction_count,
      churn_cents = bi_delta_mrr_daily.churn_cents + EXCLUDED.churn_cents,
      churn_cents_usd = bi_delta_mrr_daily.churn_cents_usd + EXCLUDED.churn_cents_usd,
      churn_count = bi_delta_mrr_daily.churn_count + EXCLUDED.churn_count,
      reactivation_cents = bi_delta_mrr_daily.reactivation_cents + EXCLUDED.reactivation_cents,
      reactivation_cents_usd = bi_delta_mrr_daily.reactivation_cents_usd + EXCLUDED.reactivation_cents_usd,
      reactivation_count = bi_delta_mrr_daily.reactivation_count + EXCLUDED.reactivation_count,
      historical_rate_id = EXCLUDED.historical_rate_id;
  RETURN NEW;
END;
$$;

create trigger tr_after_insert_bi_mrr_movement_log
  after insert
  on bi_mrr_movement_log
  for each row
execute procedure fn_update_mrr();

create or replace function fn_update_customer_ytd_summary_credit_note() returns trigger
  language plpgsql
as
$$
BEGIN
  INSERT INTO bi_customer_ytd_summary (tenant_id, customer_id, revenue_year, currency, total_revenue_cents)
  VALUES (NEW.tenant_id, NEW.customer_id, DATE_PART('year', NEW.finalized_at)::integer, NEW.currency, -NEW.refunded_amount_cents)
  ON CONFLICT (tenant_id, customer_id, currency, revenue_year) DO UPDATE
    SET total_revenue_cents = bi_customer_ytd_summary.total_revenue_cents + EXCLUDED.total_revenue_cents;
  RETURN NEW;
END;
$$;

create trigger trg_update_customer_ytd_summary_credit_note
  after insert or update
  on credit_note
  for each row
  when (new.status = 'FINALIZED'::"CreditNoteStatus")
execute procedure fn_update_customer_ytd_summary_credit_note();

create or replace function fn_update_customer_ytd_summary_invoice() returns trigger
  language plpgsql
as
$$
BEGIN
  INSERT INTO bi_customer_ytd_summary (tenant_id, customer_id, revenue_year, currency, total_revenue_cents)
  VALUES (NEW.tenant_id, NEW.customer_id,  DATE_PART('year', NEW.finalized_at)::integer, NEW.currency, NEW.amount_due)
  ON CONFLICT (tenant_id, customer_id, currency, revenue_year) DO UPDATE
    SET total_revenue_cents = bi_customer_ytd_summary.total_revenue_cents + EXCLUDED.total_revenue_cents;
  RETURN NEW;
END;
$$;

create trigger trg_update_customer_ytd_summary_invoice
  after insert or update
  on invoice
  for each row
  when (new.status = 'FINALIZED'::"InvoiceStatusEnum")
execute procedure fn_update_customer_ytd_summary_invoice();

create or replace function fn_update_revenue_credit_note() returns trigger
  language plpgsql
as
$$
DECLARE
  net_revenue_cents_usd BIGINT;
  historical_rate_record RECORD;
BEGIN

  SELECT id, (rates->>NEW.currency)::NUMERIC as rate INTO historical_rate_record
  FROM historical_rates_from_usd
  WHERE date <= NEW.finalized_at
  ORDER BY date DESC
  LIMIT 1;

  net_revenue_cents_usd := -NEW.refunded_amount_cents / historical_rate_record.rate;

  -- TODO plan_version_id is optional in invoice
  INSERT INTO bi_revenue_daily (tenant_id, plan_version_id, currency, revenue_date, net_revenue_cents, historical_rate_id, net_revenue_cents_usd)
  VALUES (NEW.tenant_id, NEW.plan_version_id, NEW.currency, DATE_TRUNC('day', NEW.finalized_at), -NEW.refunded_amount_cents, historical_rate_record.id, net_revenue_cents_usd)
  ON CONFLICT (tenant_id, plan_version_id, currency, revenue_date) DO UPDATE
    SET
      net_revenue_cents = bi_revenue_daily.net_revenue_cents + EXCLUDED.net_revenue_cents,
      net_revenue_cents_usd = bi_revenue_daily.net_revenue_cents_usd + EXCLUDED.net_revenue_cents_usd,
      historical_rate_id = EXCLUDED.historical_rate_id;
  RETURN NEW;
END;
$$;

create trigger trg_update_revenue_credit_note
  after update
  on credit_note
  for each row
  when (new.status = 'FINALIZED'::"CreditNoteStatus")
execute procedure fn_update_revenue_credit_note();

create or replace function fn_update_revenue_invoice() returns trigger
  language plpgsql
as
$$
DECLARE
  net_revenue_cents_usd BIGINT;
  historical_rate_record RECORD;
BEGIN

  SELECT id, (rates->>NEW.currency)::NUMERIC as rate INTO historical_rate_record
  FROM historical_rates_from_usd
  WHERE date <= NEW.finalized_at
  ORDER BY date DESC
  LIMIT 1;

  net_revenue_cents_usd := NEW.amount_due / historical_rate_record.rate;

  INSERT INTO bi_revenue_daily (tenant_id, plan_version_id, currency, revenue_date, net_revenue_cents, historical_rate_id, net_revenue_cents_usd)
  VALUES (NEW.tenant_id, NEW.plan_version_id, NEW.currency, DATE_TRUNC('day', NEW.finalized_at), NEW.amount_due, historical_rate_record.id, net_revenue_cents_usd)
  ON CONFLICT (tenant_id, plan_version_id, currency, revenue_date) DO UPDATE
    SET net_revenue_cents = bi_revenue_daily.net_revenue_cents + EXCLUDED.net_revenue_cents,
        net_revenue_cents_usd = bi_revenue_daily.net_revenue_cents_usd + EXCLUDED.net_revenue_cents_usd,
        historical_rate_id = EXCLUDED.historical_rate_id;
  RETURN NEW;
END;
$$;

create trigger trg_update_revenue_invoice
  after insert or update
  on invoice
  for each row
  when (new.status = 'FINALIZED'::"InvoiceStatusEnum")
execute procedure fn_update_revenue_invoice();

create or replace function convert_currency(amount numeric, source_rate_from_usd numeric, target_rate_from_usd numeric) returns numeric
  language plpgsql
as
$$
DECLARE
  conversion_rate NUMERIC;
BEGIN
  conversion_rate := target_rate_from_usd / source_rate_from_usd;
  RETURN amount * conversion_rate;
END;
$$;

create or replace function update_bi_usd_totals_from_rates() returns trigger
  language plpgsql
as
$$
BEGIN
  -- Update bi_delta_mrr_daily
  UPDATE bi_delta_mrr_daily
  SET net_mrr_cents_usd = net_mrr_cents * (NEW.rates->>currency)::NUMERIC,
      new_business_cents_usd = new_business_cents * (NEW.rates->>currency)::NUMERIC,
      expansion_cents_usd = expansion_cents * (NEW.rates->>currency)::NUMERIC,
      contraction_cents_usd = contraction_cents * (NEW.rates->>currency)::NUMERIC,
      churn_cents_usd = churn_cents * (NEW.rates->>currency)::NUMERIC,
      reactivation_cents_usd = reactivation_cents * (NEW.rates->>currency)::NUMERIC,
      historical_rate_id = NEW.id
  WHERE date = NEW.date;

  -- Update bi_revenue_daily
  UPDATE bi_revenue_daily
  SET net_revenue_cents_usd = net_revenue_cents * (NEW.rates->>currency)::NUMERIC,
      historical_rate_id = NEW.id
  WHERE revenue_date = NEW.date;

  RETURN NEW;
END;
$$;

create trigger update_usd_totals_trigger
  after insert or update
  on historical_rates_from_usd
  for each row
execute procedure update_bi_usd_totals_from_rates();
