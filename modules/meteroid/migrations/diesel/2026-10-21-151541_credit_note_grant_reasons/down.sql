UPDATE customer_credit_grant
SET reason = 'Credit returned by a credit note'
WHERE source_kind = 'CREDIT_NOTE'
  AND reason IN ('Credit pack paid after its period was invoiced', 'Credited back on plan change',
                 'Credited back on cancellation');
