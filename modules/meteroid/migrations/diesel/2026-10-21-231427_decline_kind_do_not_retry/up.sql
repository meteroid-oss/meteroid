-- A decline the issuer or card network forbids re-presenting (e.g. revoked authorization).
ALTER TYPE "DeclineKindEnum" ADD VALUE IF NOT EXISTS 'DO_NOT_RETRY';
