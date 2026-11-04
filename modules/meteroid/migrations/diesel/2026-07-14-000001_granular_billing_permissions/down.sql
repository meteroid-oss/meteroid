-- Collapse the per-area billing codes back to the umbrella ones. Lossy: a role
-- holding only a subset of billing areas widens back to all of billing.
UPDATE role
SET permissions =
        array_remove(array_remove(array_remove(
        array_remove(array_remove(array_remove(permissions,
            'billing:subscriptions:read'), 'billing:invoices:read'), 'billing:quotes:read'),
            'billing:subscriptions:manage'), 'billing:invoices:manage'), 'billing:quotes:manage')
            || CASE
                   WHEN permissions && ARRAY ['billing:subscriptions:read', 'billing:invoices:read', 'billing:quotes:read']
                       THEN ARRAY ['billing:read']
                   ELSE ARRAY []::TEXT[] END
            || CASE
                   WHEN permissions && ARRAY ['billing:subscriptions:manage', 'billing:invoices:manage', 'billing:quotes:manage']
                       THEN ARRAY ['billing:manage']
                   ELSE ARRAY []::TEXT[] END,
    updated_at  = now()
WHERE permissions && ARRAY ['billing:subscriptions:read', 'billing:invoices:read', 'billing:quotes:read',
    'billing:subscriptions:manage', 'billing:invoices:manage', 'billing:quotes:manage'];
