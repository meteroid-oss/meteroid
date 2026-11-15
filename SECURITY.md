# Security Policy

## Supported versions

Security fixes land on `main` and ship in the next release. Only the latest release is supported: we
do not backport fixes to older versions, so please upgrade to receive them.

## Reporting a vulnerability

Report vulnerabilities privately through
[GitHub private vulnerability reporting](https://github.com/meteroid-oss/meteroid/security/advisories/new).
This keeps the report, our discussion and the fix in one private advisory, and lets us credit you when
it is published.

If you cannot use GitHub, or for anything that does not fit an advisory, email
[security@meteroid.com](mailto:security@meteroid.com).

Do not report vulnerabilities in public issues, pull requests, discussions or Discord.

A useful report includes:

- the affected component and the version or commit you tested, ideally the latest release or `main`;
- the steps to reproduce it, with a proof of concept against your own instance;
- the impact you could demonstrate, and what an attacker needs beforehand (account, role, leaked
  identifier, network access).

We review every report by hand, and expect a person to have done the same before sending it:

- If you used AI tools to find the issue or write the report, please say so.
- Confirm that the finding reproduces on a current version, and that the code and behaviour the report
  describes match the repository.
- Reports must be submitted by a person who can answer follow-up questions. Reports filed
  automatically by scanners or agents, or in bulk, are closed without review.

## What to expect

- We aim to acknowledge reports on major issues within a week, and tell you whether we accept the issue once we
  have reviewed it.
- When the fix is released we publish the advisory and credit you, if you wish.
- When several people report the same issue, the first report is the primary one; later reports
  received before publication can be credited alongside it.
- We do not run a bug bounty program and do not pay for reports.

## Coordinated disclosure

Please keep the details private (no blog posts, social media, talks or public proof of concept) until
we have released a fix and published the advisory, or 90 days after your report, whichever comes
first. If you need a different timeline, let us know in the report.

## CVEs

We decide whether an advisory warrants a CVE, and request it through GitHub. We request one when a
vulnerability in Meteroid's own code affects a released version and has a meaningful impact on
self-hosted deployments. We do not request CVEs for out-of-scope issues, hardening suggestions or
duplicates.

## Scope

In scope: the code in this repository, which includes the API, the REST API, the customer portal,
metering, the workers and the web application.

Out of scope:

- Deployment examples and their defaults: the Docker Compose files, `.env.example` files, Kubernetes
  and Helm configuration under `docker/` and `k8s/`. They are starting points. Operators are
  responsible for setting their own secrets and credentials, including those of third-party services
  (PostgreSQL, ClickHouse, Kafka/Redpanda, Redis), and for restricting network access.
- Vulnerabilities in third-party dependencies. Report them to the upstream project, unless Meteroid's
  use of the dependency makes the issue exploitable.
- Issues that require an already compromised host, database, admin account or secret.
- Actions a user can take within their own tenant using permissions they were granted.
- Findings without a demonstrated security impact: missing headers, version disclosure, best-practice
  recommendations, self-XSS, clickjacking on pages without sensitive actions.
- Denial of service by volume, spam, social engineering and physical attacks.

## Testing

Please test against your own self-hosted instance, not Meteroid Cloud (`app.meteroid.com`) or any
instance you do not own, and do not access or keep data that is not yours. If you come across
someone else's data by accident, stop and mention it in your report.

We welcome good-faith research that follows this policy and will not treat it as a violation of our
terms.
