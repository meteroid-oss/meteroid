-- Seat purchases the previous release invoiced name their price component but no fee row, so
-- reconcile reads their seats as unbilled. In periods still open, each such line names the seat
-- row of its subscription in force on its start.
WITH attached AS (
    SELECT DISTINCT ON (i.id, e.ord) i.id AS invoice_id, e.ord, c.id AS component_id
    FROM invoice i
    CROSS JOIN LATERAL jsonb_array_elements(i.line_items) WITH ORDINALITY AS e(l, ord)
    JOIN subscription_component c
      ON c.subscription_id = coalesce((e.l ->> 'subscription_id')::uuid, i.subscription_id)
     AND c.price_component_id = (e.l ->> 'price_component_id')::uuid
     AND c.effective_from <= (e.l ->> 'start_date')::date
     AND (c.effective_to IS NULL OR c.effective_to > (e.l ->> 'start_date')::date)
    JOIN product p ON p.id = c.product_id AND p.fee_type = 'SLOT'
    WHERE e.l ->> 'sub_component_id' IS NULL
      AND e.l ->> 'sub_add_on_id' IS NULL
      AND coalesce(e.l ->> 'kind', 'Regular') = 'Regular'
      AND (e.l ->> 'end_date')::date >= CURRENT_DATE
    ORDER BY i.id, e.ord, c.effective_from DESC
)
UPDATE invoice i
SET line_items = (SELECT jsonb_agg(CASE WHEN a.component_id IS NULL THEN e.l
                                        ELSE e.l || jsonb_build_object('sub_component_id', a.component_id)
                                   END ORDER BY e.ord)
                  FROM jsonb_array_elements(i.line_items) WITH ORDINALITY AS e(l, ord)
                  LEFT JOIN attached a ON a.invoice_id = i.id AND a.ord = e.ord)
WHERE i.id IN (SELECT invoice_id FROM attached);
