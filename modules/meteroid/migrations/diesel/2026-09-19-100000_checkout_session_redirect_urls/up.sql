-- Merchant-set redirect targets for the hosted checkout page, per session (Stripe-style
-- success_url / cancel_url). Set through the API key, never from the customer-facing URL.
ALTER TABLE checkout_session
    ADD COLUMN success_url TEXT,
    ADD COLUMN cancel_url TEXT;
