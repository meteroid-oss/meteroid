-- The billing permission split into per-area codes so a role can e.g. manage
-- subscriptions without touching invoices. Expand the stored umbrella codes:
--   billing:read   -> billing:{subscriptions,invoices,quotes}:read
--   billing:manage -> billing:{subscriptions,invoices,quotes}:manage
-- Rows written by an old binary after this runs are expanded at parse time
-- (Permission::expand_stored_code), so this stays idempotent in effect.
UPDATE role
SET permissions =
        array_remove(array_remove(permissions, 'billing:read'), 'billing:manage')
            || CASE
                   WHEN 'billing:read' = ANY (permissions)
                       THEN ARRAY ['billing:subscriptions:read', 'billing:invoices:read', 'billing:quotes:read']
                   ELSE ARRAY []::TEXT[] END
            || CASE
                   WHEN 'billing:manage' = ANY (permissions)
                       THEN ARRAY ['billing:subscriptions:manage', 'billing:invoices:manage', 'billing:quotes:manage']
                   ELSE ARRAY []::TEXT[] END,
    updated_at  = now()
WHERE 'billing:read' = ANY (permissions)
   OR 'billing:manage' = ANY (permissions);
