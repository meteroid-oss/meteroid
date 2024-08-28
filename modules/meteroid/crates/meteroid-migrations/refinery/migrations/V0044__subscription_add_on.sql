create table if not exists subscription_add_on
(
  id                 uuid                           not null primary key,
  name               text                           not null,
  subscription_id    uuid                           not null references subscription on delete cascade,
  add_on_id          uuid                           not null references add_on on delete cascade,
  period             "SubscriptionFeeBillingPeriod" not null,
  fee                jsonb                          not null,
  created_at         timestamp(3)                   not null default current_timestamp
);

create index subscription_add_on_subscription_id_idx on subscription_add_on (subscription_id);
