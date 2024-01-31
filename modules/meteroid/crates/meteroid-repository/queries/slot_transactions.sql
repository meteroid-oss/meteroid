
--! create_slot_transaction
insert into slot_transaction(id, price_component_id, subscription_id, delta, prev_active_slots, effective_at, transaction_at)
values (:id,
        :price_component_id,
        :subscription_id,
        :delta,
        :prev_active_slots,
        :effective_at,
        :transaction_at)
returning id;

--! get_active_slots
WITH RankedSlotTransactions AS (
  SELECT
    st.*,
    ROW_NUMBER() OVER (PARTITION BY st.subscription_id, st.price_component_id ORDER BY st.transaction_at DESC) AS row_num
  FROM
    slot_transaction st
  WHERE
    st.subscription_id = :subscription_id
    AND st.price_component_id = :price_component_id
    AND st.transaction_at <= :now
)
SELECT
  X.prev_active_slots + COALESCE(SUM(Y.delta), 0) AS current_active_slots
FROM
  RankedSlotTransactions X
    LEFT JOIN
  RankedSlotTransactions Y ON Y.effective_at BETWEEN X.transaction_at AND :now
WHERE
  X.row_num = 1
GROUP BY
  X.prev_active_slots;
