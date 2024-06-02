create type "BillingCycleEnum" as enum ('FOREVER', 'FIXED');
create type "BillingMetricAggregateEnum" as enum ('COUNT', 'LATEST', 'MAX', 'MIN', 'MEAN', 'SUM', 'COUNT_DISTINCT');
create type "UnitConversionRoundingEnum" as enum ('UP', 'DOWN', 'NEAREST', 'NEAREST_HALF', 'NEAREST_DECILE', 'NONE');
create type "OrganizationUserRole" as enum ('ADMIN', 'MEMBER');
create type "BillingFrequencyEnum" as enum ('ANNUAL', 'MONTHLY');
create type "ServicePeriodStartOnEnum" as enum ('DAY_OF_MONTH', 'START_OF_PLAN');

create table organization
(
  id          uuid                                   not null
    primary key,
  name        text                                   not null,
  slug        text                                   not null,
  created_at  timestamp(3) default CURRENT_TIMESTAMP not null,
  archived_at timestamp(3)
);

create unique index organization_slug_key on organization (slug);

create table "user"
(
  id          uuid                                   not null
    primary key,
  email       text                                   not null,
  created_at  timestamp(3) default CURRENT_TIMESTAMP not null,
  archived_at timestamp(3)
);

create unique index user_email_key on "user" (email);

create table tenant
(
  id              uuid                                   not null
    primary key,
  name            text                                   not null,
  slug            text                                   not null,
  created_at      timestamp(3) default CURRENT_TIMESTAMP not null,
  updated_at      timestamp(3),
  archived_at     timestamp(3),
  organization_id uuid                                   not null
    references organization
      on update cascade on delete restrict,
  invite_link_id  uuid,
  billing_config  jsonb
);

create table customer
(
  id             uuid                                   not null
    primary key,
  name           text                                   not null,
  aliases        text[],
  created_at     timestamp(3) default CURRENT_TIMESTAMP not null,
  created_by     uuid                                   not null,
  updated_at     timestamp(3),
  updated_by     uuid,
  archived_at    timestamp(3),
  tenant_id      uuid                                   not null
    references tenant
      on update cascade on delete restrict,
  billing_config jsonb
);

create table product_family
(
  id          uuid                                   not null
    primary key,
  name        text                                   not null,
  api_name    text                                   not null,
  created_at  timestamp(3) default CURRENT_TIMESTAMP not null,
  updated_at  timestamp(3),
  archived_at timestamp(3),
  tenant_id   uuid                                   not null
    references tenant
      on update cascade on delete restrict
);

create table billable_metric
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

create table plan
(
  id                  uuid                                   not null
    primary key,
  name                text                                   not null,
  api_name            text                                   not null,
  description         text,
  created_at          timestamp(3) default CURRENT_TIMESTAMP not null,
  created_by          uuid                                   not null,
  updated_at          timestamp(3),
  archived_at         timestamp(3),
  tenant_id           uuid                                   not null
    references tenant
      on update cascade on delete restrict,
  product_family_id   uuid                                   not null
    references product_family
      on update cascade on delete restrict,
  is_free             boolean                                not null,
  trial_duration_days integer
);

create unique index plan_tenant_id_api_name_key on plan (tenant_id, api_name);

create table price_point
(
  id           uuid                       not null
    primary key,
  name         text                       not null,
  currency     text                       not null,
  frequency    "BillingFrequencyEnum"     not null,
  cycle        "BillingCycleEnum"         not null,
  period_start "ServicePeriodStartOnEnum" not null,
  plan_id      uuid                       not null
    references plan
      on update cascade on delete restrict,
  net_terms    integer                    not null,
  day_of_month smallint default 1         not null
);

create table price_ramp
(
  id                          uuid not null
    primary key,
  name                        text not null,
  price_point_id              uuid not null
    references price_point
      on update cascade on delete restrict,
  discount                    jsonb,
  minimum                     jsonb,
  free_credit                 jsonb,
  duration_in_billing_periods integer,
  idx                         integer
);

create table product
(
  id                uuid                                   not null
    primary key,
  name              text                                   not null,
  description       text,
  tags              text[],
  group_key         text,
  created_at        timestamp(3) default CURRENT_TIMESTAMP not null,
  created_by        uuid                                   not null,
  updated_at        timestamp(3),
  archived_at       timestamp(3),
  tenant_id         uuid                                   not null
    references tenant
      on update cascade on delete restrict,
  product_family_id uuid
                                                           references product_family
                                                             on update cascade on delete set null
);

create table priced_product
(
  id                     uuid not null
    primary key,
  custom_pricing_unit_id uuid,
  discount               jsonb,
  minimum                jsonb,
  usage_based_fees       jsonb,
  recurring_fees         jsonb,
  product_id             uuid not null
    references product
      on update cascade on delete restrict,
  price_point_id         uuid not null
    references price_point
      on update cascade on delete restrict
);

create table product_charge
(
  id          uuid                                   not null
    primary key,
  name        text                                   not null,
  description text,
  created_at  timestamp(3) default CURRENT_TIMESTAMP not null,
  created_by  uuid                                   not null,
  updated_at  timestamp(3),
  archived_at timestamp(3),
  product_id  uuid                                   not null
    references product
      on update cascade on delete restrict
);

create unique index product_family_api_name_tenant_id_key on product_family (api_name, tenant_id);

create table subscription
(
  id                 uuid     not null
    primary key,
  customer_id        uuid     not null
    references customer
      on update cascade on delete restrict,
  billing_day        smallint not null,
  plan_id            uuid     not null
    references plan
      on update cascade on delete restrict,
  price_point_id     uuid     not null
    references price_point
      on update cascade on delete restrict,
  tenant_id          uuid     not null
    references tenant
      on update cascade on delete restrict,
  trial_start_date   date,
  billing_start_date date     not null,
  billing_end_date   date
);

create index subscription_billing_day_idx
  on subscription (billing_day);

create index subscription_start_date_idx
  on subscription (billing_start_date);

create index subscription_end_date_idx
  on subscription (billing_end_date);

create unique index tenant_slug_key
  on tenant (slug);

create unique index tenant_name_organization_id_key
  on tenant (name, organization_id);

create table tenant_invite
(
  id         uuid                                   not null
    primary key,
  email      text                                   not null,
  created_at timestamp(3) default CURRENT_TIMESTAMP not null,
  tenant_id  uuid                                   not null
    references tenant
      on update cascade on delete restrict
);

create table tenant_invite_link
(
  id         uuid                                   not null
    primary key,
  created_at timestamp(3) default CURRENT_TIMESTAMP not null,
  tenant_id  uuid                                   not null
    references tenant
      on update cascade on delete restrict
);

create unique index tenant_invite_link_tenant_id_key
  on tenant_invite_link (tenant_id);

create table api_token
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

create unique index api_token_hash_key
  on api_token (hash);

create table organization_member
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

create table billable_metric_to_product
(
  billable_metric_id uuid not null
    references billable_metric
      on update cascade on delete restrict,
  product_id         uuid not null
    references product
      on update cascade on delete restrict,
  primary key (product_id, billable_metric_id)
);

create table plan_to_product
(
  plan_id    uuid not null
    references plan
      on update cascade on delete restrict,
  product_id uuid not null
    references product
      on update cascade on delete restrict,
  primary key (product_id, plan_id)
);
