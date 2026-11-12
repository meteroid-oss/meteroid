-- Ephemeral licenses (CI, short-lived installations): the activation code is redeemed any number
-- of times, each activation signs a license of this many hours for the calling installation, and
-- nothing is refreshed.
ALTER TABLE issued_license
    ADD COLUMN ephemeral_hours INT CHECK (ephemeral_hours BETWEEN 1 AND 24);
