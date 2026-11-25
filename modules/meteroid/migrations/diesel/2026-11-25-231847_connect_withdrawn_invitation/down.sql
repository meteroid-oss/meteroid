DO $$
BEGIN
    IF to_regclass('connect_connected_account') IS NOT NULL THEN
        ALTER TABLE connect_connected_account DROP CONSTRAINT IF EXISTS chk_express_or_connected;
        ALTER TABLE connect_connected_account
            ADD CONSTRAINT chk_express_or_connected CHECK (
                (connection_type = 'STANDARD' AND connected_organization_id IS NOT NULL) OR
                (connection_type = 'EXPRESS' AND (
                    (status = 'PENDING' AND connected_organization_id IS NULL AND pending_email IS NOT NULL) OR
                    (status != 'PENDING' AND connected_organization_id IS NOT NULL)
                ))
            );
    END IF;
END
$$;
