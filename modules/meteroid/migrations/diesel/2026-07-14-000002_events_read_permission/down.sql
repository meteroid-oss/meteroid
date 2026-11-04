-- Drop the dedicated events capability; event visibility reverts to settings:read.
UPDATE role
SET permissions = array_remove(permissions, 'events:read'),
    updated_at  = now()
WHERE 'events:read' = ANY (permissions);
