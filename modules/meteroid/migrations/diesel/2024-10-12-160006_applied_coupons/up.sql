drop table if exists subscription_coupon;

alter table coupon alter column recurring_value drop not null;
alter table coupon add constraint positive_recurring_value check (recurring_value is null or recurring_value > 0);
alter table coupon add constraint positive_redemption_limit check (redemption_limit is null or redemption_limit > 0);
alter table coupon add column if not exists redemption_count integer not null default 0;
alter table coupon add column if not exists last_redemption_at timestamp(3);
alter table coupon add column if not exists disabled boolean not null default false;
alter table coupon add column if not exists archived_at timestamp(3);

create unique index if not exists coupon_tenant_id_code_idx
  on coupon (tenant_id, code) where archived_at is null;

create table if not exists applied_coupon
(
  id              uuid primary key,
  coupon_id       uuid         not null references coupon on delete cascade,
  customer_id     uuid         not null references customer on delete cascade,
  subscription_id uuid         not null references subscription on delete cascade,
  is_active       boolean      not null,
  applied_amount  numeric,
  applied_count   integer,
  last_applied_at timestamp(3),
  created_at      timestamp(3) not null default now()
);

alter table applied_coupon add constraint positive_applied_amount check (applied_amount is null or applied_amount > 0);
alter table applied_coupon add constraint positive_applied_count check (applied_count is null or applied_count > 0);

create unique index if not exists applied_coupon_subscription_id_coupon_id_idx
  on applied_coupon (subscription_id, coupon_id);

create index if not exists applied_coupon_coupon_id_idx
  on applied_coupon (coupon_id);

create index if not exists applied_coupon_customer_id_coupon_id_idx
  on applied_coupon (customer_id, coupon_id);

alter table invoice add column applied_invoice_ids uuid[] not null default '{}';
