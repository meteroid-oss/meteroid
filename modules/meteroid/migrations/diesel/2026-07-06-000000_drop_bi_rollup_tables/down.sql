-- Recreate the BI rollup tables in their final (pre-drop) shape: the USD money
-- columns are NUMERIC(20,4) since 2026-01-17-110000_drop_bi_revenue_triggers.
-- Data is NOT restored; the tables come back empty.

create table if not exists bi_customer_ytd_summary
(
  tenant_id           uuid    not null,
  customer_id         uuid    not null,
  revenue_year        integer not null,
  currency            text    not null,
  total_revenue_cents bigint  not null,
  primary key (tenant_id, customer_id, currency, revenue_year)
);

create table if not exists bi_delta_mrr_daily
(
  tenant_id              uuid          not null,
  plan_version_id        uuid          not null,
  date                   date          not null,
  currency               text          not null,
  net_mrr_cents          bigint        not null,
  new_business_cents     bigint        not null,
  new_business_count     integer       not null,
  expansion_cents        bigint        not null,
  expansion_count        integer       not null,
  contraction_cents      bigint        not null,
  contraction_count      integer       not null,
  churn_cents            bigint        not null,
  churn_count            integer       not null,
  reactivation_cents     bigint        not null,
  reactivation_count     integer       not null,
  historical_rate_id     uuid          not null
    references historical_rates_from_usd
      on update cascade on delete restrict,
  net_mrr_cents_usd      numeric(20, 4) not null,
  new_business_cents_usd numeric(20, 4) not null,
  expansion_cents_usd    numeric(20, 4) not null,
  contraction_cents_usd  numeric(20, 4) not null,
  churn_cents_usd        numeric(20, 4) not null,
  reactivation_cents_usd numeric(20, 4) not null,
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
  net_revenue_cents_usd numeric(20, 4)                 not null,
  id                    uuid default gen_random_uuid() not null
    primary key
);

create unique index if not exists bi_revenue_daily_uniqueness
  on bi_revenue_daily (tenant_id, plan_version_id, currency, revenue_date);

SELECT pgmq.create('bi_aggregation');
