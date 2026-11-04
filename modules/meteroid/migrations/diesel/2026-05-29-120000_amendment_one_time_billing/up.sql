-- Distinguish components / add-ons added by a manual amendment from those that
-- come from the plan definition or a plan change. A one-time fee added via an
-- amendment must be billed on the invoice for the period it becomes effective
-- (subscription start no longer being its only billing point), whereas a plan's
-- own one-time fee (e.g. a setup fee carried by a plan change) must NOT be
-- re-billed when the plan changes. Recurring fees are unaffected by this flag.

ALTER TABLE subscription_component
  ADD COLUMN added_by_amendment BOOLEAN NOT NULL DEFAULT false;

ALTER TABLE subscription_add_on
  ADD COLUMN added_by_amendment BOOLEAN NOT NULL DEFAULT false;
