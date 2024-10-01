create table if not exists subscription_coupon
(
  id                      uuid primary key,
  subscription_id         uuid         not null references subscription on
    delete cascade,
  coupon_id               uuid         not null references coupon
    on
      delete restrict,
  coupon_code             text         not null,
  coupon_description      text         not null,
  coupon_discount         jsonb        not null,
  coupon_expires_at       timestamp(3),
  coupon_redemption_limit integer,
  coupon_recurring_value  integer      not null,
  coupon_reusable         boolean      not null,
  created_at              timestamp(3) not null default now()
);

create index if not exists subscription_coupon_subscription_id_idx
  on subscription_coupon (subscription_id);
