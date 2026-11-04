-- Raw usage events moved off settings:read onto a dedicated events:read.
-- Grant events:read to every custom role that could previously see events
-- (i.e. held settings:read, or settings:manage which implies read) so no role
-- silently loses the events explorer. Admins can now remove it from roles that
-- should not inspect usage, or grant it alone without settings visibility.
UPDATE role
SET permissions = array_append(permissions, 'events:read'),
    updated_at  = now()
WHERE ('settings:read' = ANY (permissions) OR 'settings:manage' = ANY (permissions))
  AND NOT ('events:read' = ANY (permissions));
