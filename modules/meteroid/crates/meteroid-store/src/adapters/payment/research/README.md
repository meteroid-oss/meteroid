# Payment provider research docs

Payment provider research documents may be created during integration development
to document the official sources, capability matrix, request map, webhook map,
and recurring + one-off integration flows a `<provider>.rs` adapter is built
against.

Existing adapters (Stripe, GoCardless) were implemented without committed research
docs in this directory. Older docs may name provider-specific return routes and
`<provider>_status` markers; every hosted-redirect provider now returns through
`/v1/portal/hosted/return` with `hosted_status` / `hosted_error`. Future provider integrations are encouraged to use the
`add-payment-provider` skill (`.claude/skills/add-payment-provider/`) to produce
and commit a research doc before implementation.
