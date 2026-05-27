-- Customer-facing action required to complete a charge (3DS / SCA). When set
-- (and status is still Pending), the payment is waiting for the customer to
-- authenticate; the portal reads it to drive Stripe.js. Cleared on terminal.
ALTER TABLE payment_transaction
    ADD COLUMN next_action JSONB;
