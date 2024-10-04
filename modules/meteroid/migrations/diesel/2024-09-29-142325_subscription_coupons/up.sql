create table if not exists subscription_coupon
(
  id                      uuid primary key,
  subscription_id         uuid         not null references subscription on
    delete cascade,
  coupon_id               uuid         not null references coupon
    on
      delete restrict,
  created_at              timestamp(3) not null default now()
);

create index if not exists subscription_coupon_subscription_id_idx
  on subscription_coupon (subscription_id);

create index if not exists subscription_coupon_coupon_id_idx
  on subscription_coupon (coupon_id);
