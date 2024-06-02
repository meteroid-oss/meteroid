create type "SubscriptionStatusEnum" as enum ('PENDING', 'ACTIVE', 'CANCELLED');
alter table subscription add column status "SubscriptionStatusEnum" default 'PENDING'::"SubscriptionStatusEnum" not null;
