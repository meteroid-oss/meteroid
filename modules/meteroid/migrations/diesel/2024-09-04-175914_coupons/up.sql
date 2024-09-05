create table if not exists coupon
(
  id               uuid                                   not null primary key,
  code             text                                   not null,
  description      text                                   not null,
  tenant_id        uuid                                   not null references tenant on update cascade on delete restrict,
  discount         jsonb                                  not null,
  expires_at       timestamp(3),
  redemption_limit integer,
  recurring_value  integer                                not null,
  reusable         boolean                                not null,
  created_at       timestamp(3) default CURRENT_TIMESTAMP not null,
  updated_at       timestamp(3) default CURRENT_TIMESTAMP not null
);

create index if not exists coupon_tenant_id_idx on coupon (tenant_id);
