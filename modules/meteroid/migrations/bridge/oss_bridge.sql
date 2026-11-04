-- Legacy-OSS migration bridge: moves a database migrated by the public repository (up to legacy-final) onto the
-- enterprise migration history. Runs before Diesel on every startup and does nothing once that is done, or on any
-- other database. See modules/meteroid/migrations/bridge/UPGRADING_FROM_LEGACY_OSS.md. One DO statement: atomic.
DO $bridge$
DECLARE
    -- Public version, enterprise version. Never edit: both histories are frozen.
    mapping CONSTANT TEXT[] := ARRAY[
        ['202603272117360000', '202602022117360000'], -- remove_bm_sync_queue
        ['20260611000000',     '20260515000000'],     -- entity_activity (the public one is consolidated:
        ['20260611000000',     '20260522000000'],     --   drop_created_by_columns
        ['20260611000000',     '20260606000000'],     --   entity_activity_actor_split)
        ['20260611120000',     '20260526120000'],     -- subscription_amendment
        ['20260611120001',     '20260529120000'],     -- amendment_one_time_billing
        ['20260630120000',     '20260614120000'],     -- feature_code (same statements, reformatted)
        ['20260820000004',     '20260715110000'],     -- payment_transaction_initiator
        ['20260821000000',     '20260828100863'],     -- stancer_payment_provider
        ['20260821000001',     '20260828101089'],     -- payment_transaction_pending_intent
        ['20260827220329',     '20260828102119']      -- tax_model
    ];
    legacy BOOLEAN;
    connect_placeholder BOOLEAN;
    unmapped TEXT[];
    missing TEXT;
BEGIN
    IF to_regclass('__diesel_schema_migrations') IS NULL THEN
        RETURN; -- fresh database: Diesel applies everything
    END IF;

    PERFORM pg_advisory_xact_lock(hashtext('meteroid.migrations'));

    -- Versions no enterprise migration has. 20260821000000 is not one of them: on an enterprise
    -- database it is entity_activity_user_actor_index, not the public stancer_payment_provider.
    legacy := EXISTS (SELECT 1 FROM __diesel_schema_migrations
                      WHERE version IN ('202603272117360000', '20260611000000', '20260611120000', '20260611120001',
                                        '20260630120000', '20260820000004', '20260821000001', '20260827220329'));
    -- The public 20260312000001_meteroid_connect was a placeholder (two columns, no FK, no tables).
    connect_placeholder := to_regclass('connect_oauth_app') IS NULL
        AND EXISTS (SELECT 1 FROM __diesel_schema_migrations WHERE version = '20260312000001');
    IF NOT legacy AND NOT connect_placeholder THEN
        RETURN; -- enterprise or Cloud database, or a pre-2026-03-27 public one already bridged
    END IF;

    IF legacy THEN
        -- Recorded public versions whose enterprise versions are not all recorded yet.
        SELECT array_agg(DISTINCT mapping[i][1]) INTO unmapped
        FROM generate_subscripts(mapping, 1) AS i
        WHERE EXISTS (SELECT 1 FROM __diesel_schema_migrations WHERE version = mapping[i][1])
          AND NOT EXISTS (SELECT 1 FROM __diesel_schema_migrations WHERE version = mapping[i][2]);
    END IF;

    -- Refuse to record a migration whose changes are not in the schema; nothing is written then.
    -- Only versions about to be recorded: once they are, later migrations may change these objects.
    IF unmapped IS NOT NULL THEN
        SELECT string_agg(c.public_version, ', ' ORDER BY c.public_version) INTO missing
        FROM (VALUES
            ('202603272117360000', NOT EXISTS (SELECT 1 FROM pg_attribute WHERE attrelid = to_regclass('billable_metric')
                                               AND attname = 'synced_at' AND NOT attisdropped)),
            ('20260611000000', to_regclass('sent_email') IS NOT NULL
                AND EXISTS (SELECT 1 FROM pg_attribute WHERE attrelid = to_regclass('entity_activity')
                            AND attname = 'actor_uuid' AND NOT attisdropped)
                AND NOT EXISTS (SELECT 1 FROM pg_attribute WHERE attrelid = to_regclass('entity_activity')
                                AND attname = 'actor_id' AND NOT attisdropped)
                AND NOT EXISTS (SELECT 1 FROM pg_attribute WHERE attrelid = to_regclass('customer')
                                AND attname = 'created_by' AND NOT attisdropped)
                AND EXISTS (SELECT 1 FROM pg_constraint WHERE conrelid = to_regclass('entity_activity')
                            AND conname = 'entity_activity_actor_shape')),
            ('20260611120000', EXISTS (SELECT 1 FROM pg_attribute WHERE attrelid = to_regclass('subscription_add_on')
                                       AND attname = 'lineage_id' AND NOT attisdropped)
                AND EXISTS (SELECT 1 FROM pg_attribute WHERE attrelid = to_regclass('scheduled_event')
                            AND attname = 'created_by_customer' AND NOT attisdropped)),
            ('20260611120001', EXISTS (SELECT 1 FROM pg_attribute WHERE attrelid = to_regclass('subscription_component')
                                       AND attname = 'added_by_amendment' AND NOT attisdropped)),
            ('20260630120000', EXISTS (SELECT 1 FROM pg_constraint WHERE conrelid = to_regclass('feature')
                                       AND conname = 'feature_tenant_code_key')),
            ('20260820000004', EXISTS (SELECT 1 FROM pg_attribute WHERE attrelid = to_regclass('payment_transaction')
                                       AND attname = 'initiated_by_customer_id' AND NOT attisdropped)),
            ('20260821000000', EXISTS (SELECT 1 FROM pg_enum WHERE enumtypid = to_regtype('"ConnectorProviderEnum"')
                                       AND enumlabel = 'STANCER')),
            ('20260821000001', EXISTS (SELECT 1 FROM pg_attribute WHERE attrelid = to_regclass('payment_transaction')
                                       AND attname = 'pending_provider_intent_id' AND NOT attisdropped)),
            ('20260827220329', to_regclass('tax_category') IS NOT NULL
                AND EXISTS (SELECT 1 FROM pg_attribute WHERE attrelid = to_regclass('customer')
                            AND attname = 'tax_status' AND NOT attisdropped))
        ) AS c (public_version, ok)
        WHERE c.public_version = ANY (unmapped) AND NOT c.ok;
        IF missing IS NOT NULL THEN
            RAISE EXCEPTION 'oss migration bridge: version(s) % are recorded but their schema changes are missing; nothing was changed', missing;
        END IF;
    END IF;

    -- Create what the enterprise 20260312000001_meteroid_connect creates, as it created it.
    IF connect_placeholder THEN
        IF EXISTS (SELECT 1 FROM customer WHERE connected_account_id IS NOT NULL) THEN
            RAISE EXCEPTION 'oss migration bridge: customer.connected_account_id is set on some rows but no Connect account exists; nothing was changed';
        END IF;

        -- Enum types for Connect
        CREATE TYPE "ConnectConnectionTypeEnum" AS ENUM ('STANDARD', 'EXPRESS');
        CREATE TYPE "ConnectConnectionStatusEnum" AS ENUM ('PENDING', 'ACTIVE', 'REVOKED', 'SUSPENDED');
        CREATE TYPE "ConnectOnboardingModeEnum" AS ENUM ('EXPRESS', 'FULL');

        -- OAuth Applications (registered by platforms)
        CREATE TABLE connect_oauth_app (
            id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
            organization_id UUID NOT NULL REFERENCES organization(id),
            name TEXT NOT NULL,
            client_id TEXT NOT NULL UNIQUE,
            client_secret_hash TEXT NOT NULL,
            client_secret_hint TEXT NOT NULL,
            redirect_uris TEXT[] NOT NULL,
            scopes TEXT[] NOT NULL DEFAULT '{"connect:full"}',
            is_active BOOLEAN NOT NULL DEFAULT true,
            created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            updated_at TIMESTAMPTZ
        );

        -- client_id already has an index from the UNIQUE constraint
        CREATE INDEX idx_connect_oauth_app_organization ON connect_oauth_app(organization_id);

        -- Authorization Codes (short-lived, for OAuth code exchange)
        CREATE TABLE connect_authorization_code (
            id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
            code_hash TEXT NOT NULL UNIQUE,
            oauth_app_id UUID NOT NULL REFERENCES connect_oauth_app(id) ON DELETE CASCADE,
            connected_organization_id UUID NOT NULL REFERENCES organization(id),
            redirect_uri TEXT NOT NULL,
            scopes TEXT[] NOT NULL,
            code_challenge TEXT,
            code_challenge_method TEXT,
            expires_at TIMESTAMPTZ NOT NULL,
            created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            used_at TIMESTAMPTZ
        );

        -- code_hash already has an index from the UNIQUE constraint
        CREATE INDEX idx_connect_auth_code_expires ON connect_authorization_code(expires_at);
        CREATE INDEX idx_connect_auth_code_oauth_app ON connect_authorization_code(oauth_app_id);

        -- Access Tokens
        CREATE TABLE connect_access_token (
            id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
            token_hash TEXT NOT NULL UNIQUE,
            oauth_app_id UUID NOT NULL REFERENCES connect_oauth_app(id) ON DELETE CASCADE,
            connected_organization_id UUID NOT NULL REFERENCES organization(id),
            scopes TEXT[] NOT NULL,
            expires_at TIMESTAMPTZ NOT NULL,
            created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            revoked_at TIMESTAMPTZ
        );

        -- token_hash already has an index from the UNIQUE constraint
        CREATE INDEX idx_connect_access_token_expires ON connect_access_token(expires_at);
        CREATE INDEX idx_connect_access_token_oauth_app ON connect_access_token(oauth_app_id);
        CREATE INDEX idx_connect_access_token_org ON connect_access_token(connected_organization_id);

        -- Refresh Tokens
        CREATE TABLE connect_refresh_token (
            id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
            token_hash TEXT NOT NULL UNIQUE,
            access_token_id UUID NOT NULL REFERENCES connect_access_token(id) ON DELETE CASCADE,
            expires_at TIMESTAMPTZ,
            created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            revoked_at TIMESTAMPTZ,
            rotated_to_id UUID REFERENCES connect_refresh_token(id)
        );

        -- token_hash already has an index from the UNIQUE constraint
        CREATE INDEX idx_connect_refresh_token_access ON connect_refresh_token(access_token_id);

        -- Connected Accounts (platform <-> organization relationships)
        -- connected_organization_id is nullable for express accounts (created before org exists)
        CREATE TABLE connect_connected_account (
            id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
            platform_organization_id UUID NOT NULL REFERENCES organization(id),
            connected_organization_id UUID REFERENCES organization(id),
            connection_type "ConnectConnectionTypeEnum" NOT NULL,
            status "ConnectConnectionStatusEnum" NOT NULL DEFAULT 'PENDING',
            onboarding_mode "ConnectOnboardingModeEnum" NOT NULL DEFAULT 'EXPRESS',
            onboarding_completed_at TIMESTAMPTZ,
            metadata JSONB,
            created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            revoked_at TIMESTAMPTZ,
            -- Express flow pending info
            pending_email TEXT,
            pending_organization_name TEXT,
            pending_country TEXT,
            platform_customer_id UUID UNIQUE REFERENCES customer(id),
            connected_tenant_id UUID REFERENCES tenant(id),
            UNIQUE(platform_organization_id, connected_organization_id)
        );

        -- Standard accounts require connected_organization_id, express pending accounts don't
        ALTER TABLE connect_connected_account
            ADD CONSTRAINT chk_express_or_connected CHECK (
                (connection_type = 'STANDARD' AND connected_organization_id IS NOT NULL) OR
                (connection_type = 'EXPRESS' AND (
                    (status = 'PENDING' AND connected_organization_id IS NULL AND pending_email IS NOT NULL) OR
                    (status != 'PENDING' AND connected_organization_id IS NOT NULL)
                ))
            );

        CREATE INDEX idx_connect_connected_account_platform ON connect_connected_account(platform_organization_id);
        CREATE INDEX idx_connect_connected_account_connected ON connect_connected_account(connected_organization_id);
        CREATE INDEX idx_connect_connected_account_status ON connect_connected_account(status);
        CREATE INDEX idx_connect_connected_account_pending_email
            ON connect_connected_account(pending_email)
            WHERE pending_email IS NOT NULL;

        CREATE UNIQUE INDEX idx_connect_connected_account_pending_unique
            ON connect_connected_account(platform_organization_id, pending_email)
            WHERE status = 'PENDING';

        ALTER TABLE customer
            ADD CONSTRAINT customer_connected_account_id_fkey
            FOREIGN KEY (connected_account_id) REFERENCES connect_connected_account(id);

        CREATE TABLE connect_platform_settings (
            organization_id UUID PRIMARY KEY REFERENCES organization(id),
            is_platform_enabled BOOLEAN NOT NULL DEFAULT false,
            default_scopes TEXT[] NOT NULL DEFAULT '{"connect:full"}',
            branding_invoicing_entity_id UUID REFERENCES invoicing_entity(id),
            created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
        );

        CREATE TABLE connect_onboarding_link (
            id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
            connected_account_id UUID NOT NULL REFERENCES connect_connected_account(id) ON DELETE CASCADE,
            token_hash TEXT NOT NULL UNIQUE,
            redirect_url TEXT NOT NULL,
            expires_at TIMESTAMPTZ NOT NULL,
            used_at TIMESTAMPTZ,
            created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
        );

        CREATE INDEX idx_connect_onboarding_link_expires ON connect_onboarding_link(expires_at);
        CREATE INDEX idx_connect_onboarding_link_account ON connect_onboarding_link(connected_account_id);
    END IF;

    -- Record the enterprise equivalents of the applied public versions. The public rows stay, so a
    -- legacy binary still running during the rollout finds nothing pending.
    IF unmapped IS NOT NULL THEN
        INSERT INTO __diesel_schema_migrations (version, run_on)
        SELECT mapping[i][2], d.run_on
        FROM generate_subscripts(mapping, 1) AS i
        JOIN __diesel_schema_migrations d ON d.version = mapping[i][1]
        WHERE mapping[i][1] = ANY (unmapped)
        ON CONFLICT (version) DO NOTHING;
    END IF;

    -- These two versions belong to public migrations here, so Diesel takes the enterprise migrations of
    -- the same version as applied: apply them. Checked in the catalog first so later runs lock nothing.
    IF legacy THEN
        IF EXISTS (SELECT 1 FROM __diesel_schema_migrations WHERE version = '20260708000000')
           AND NOT EXISTS (SELECT 1 FROM pg_attribute WHERE attrelid = to_regclass('"user"')
                           AND attname = 'preferences' AND NOT attisdropped) THEN
            -- 2026-07-08-000000_user_preferences
            ALTER TABLE "user"
                ADD COLUMN IF NOT EXISTS preferences JSONB NOT NULL DEFAULT '{}'::jsonb;
        END IF;

        IF EXISTS (SELECT 1 FROM __diesel_schema_migrations WHERE version = '20260821000000')
           AND to_regclass('idx_entity_activity_user_created') IS NULL THEN
            -- 2026-08-21-000000_entity_activity_user_actor_index, without CONCURRENTLY (not allowed in a transaction)
            CREATE INDEX IF NOT EXISTS idx_entity_activity_user_created
                ON entity_activity (activity_type, entity_type, tenant_id, entity_id)
                WHERE actor_type <> 'SYSTEM';
        END IF;
    END IF;
END
$bridge$;
