-- Balance funded by a system credit note is named by why it was issued.
UPDATE customer_credit_grant g
SET reason = CASE cn.reason
    WHEN 'Credit pack paid after its period was invoiced' THEN 'Credit pack paid after its period was invoiced'
    WHEN 'Plan change' THEN 'Credited back on plan change'
    WHEN 'Subscription cancellation' THEN 'Credited back on cancellation'
END
FROM credit_note cn
WHERE cn.id = g.credit_note_id
  AND cn.tenant_id = g.tenant_id
  AND g.source_kind = 'CREDIT_NOTE'
  AND g.reason = 'Credit returned by a credit note'
  AND cn.reason IN ('Credit pack paid after its period was invoiced', 'Plan change', 'Subscription cancellation');
