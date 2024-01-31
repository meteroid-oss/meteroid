
create table if not exists slot_transaction
(
  id                      uuid                                   not null primary key,
  price_component_id      uuid                                   not null references price_component on delete restrict on update cascade,
  subscription_id         uuid                                   not null references subscription on delete restrict on update cascade,
  delta                   integer                                not null,
  prev_active_slots       integer                                not null,
  effective_at            timestamp(3)                           not null,
  transaction_at          timestamp(3) default CURRENT_TIMESTAMP not null
);

create index if not exists slot_transaction_sub_price_comp_idx on slot_transaction (subscription_id, price_component_id);
