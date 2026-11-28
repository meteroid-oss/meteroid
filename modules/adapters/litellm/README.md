# meteroid-litellm

Bill the usage of a [LiteLLM](https://github.com/BerriAI/litellm) proxy with Meteroid: every
successful call becomes usage events on your customers, batched and idempotent, and an optional
pre-call gate refuses the calls of customers whose prepaid credit ran out.

- **Usage metadata only.** Model, provider, token counts, the call id and the cost LiteLLM
  computed. Never the messages, the response, keys or IP addresses.
- **Billed once.** Each call yields one event per token class, its id `<litellm_call_id>:<class>`:
  a batch sent again after a timeout or a `5xx` is deduplicated by Meteroid.
- **Never in the way.** Events are sent in the background; when Meteroid is unreachable they wait
  in a bounded queue, and the gate lets calls through after its timeout.

MIT licensed, a standalone package: it copies no code from LiteLLM or Meteroid.

## 15-minute quickstart

You need a Meteroid instance (Meteroid Cloud, or self-hosted with `docker compose` from
`docker/deploy`) and a LiteLLM proxy (tested with LiteLLM 1.111.0, the `main-stable` image pinned in
`tests/e2e/docker-compose.yml`).

### 1. Bill tokens in Meteroid (5 minutes)

In Meteroid, in a sandbox tenant:

1. **Metric.** Product catalog → Metrics → New metric: code `llm_tokens`, aggregation *Sum* of
   the property `tokens`.
2. **Prices.** On the metric, a rate card by `model` and `token_class`, per 1M tokens, e.g.
   `gpt-4.1 · input` 2.50, `gpt-4.1 · output` 10, and `* · *` for any other model. Price a plan
   with it (Plans → New plan → a usage charge rated by the card).
3. **Customer.** Create a customer whose **alias** is the identity LiteLLM sees for it (the
   customer organization alias, the end user `user` field, or the team alias), and subscribe it
   to the plan.
4. **API key.** Developers → API keys → New key.

With the Enterprise AI module, turn it on in Settings → AI and add a **cost card** in AI → Cost
book (what each model costs you, per 1M tokens): AI → Margin then shows revenue, cost and margin
per customer and model.

### 2. Install the callback in the proxy (5 minutes)

```bash
pip install ./modules/adapters/litellm   # from a Meteroid checkout, in the proxy's image or virtualenv
export METEROID_API_KEY=<the API key>
export METEROID_API_BASE=https://api.meteroid.com   # or your instance's REST API, e.g. http://meteroid-api:8084
```

```yaml
# config.yaml
litellm_settings:
  callbacks: ["meteroid_litellm.handler"]
```

Restart the proxy. To try it without a provider key, add a model whose `api_base` is any
OpenAI-compatible mock, as `tests/e2e/docker-compose.yml` does.

### 3. Send traffic and see the invoice (5 minutes)

```bash
curl -s http://localhost:4000/v1/chat/completions \
  -H "Authorization: Bearer $LITELLM_MASTER_KEY" -H "Content-Type: application/json" \
  -d '{"model": "gpt-4.1", "user": "acme-gmbh", "messages": [{"role": "user", "content": "Hello"}]}'
```

Within the flush interval (5 s) the events appear in Meteroid under Events, then on the
customer's usage and its draft invoice, a line per model and token class.

## Events

One event per token class with tokens, on the code `METEROID_EVENT_CODE` (default `llm_tokens`):

| Property | Value |
|---|---|
| `model` | the model group the client asked for (`model_group`), else the model |
| `provider` | LiteLLM's `custom_llm_provider` |
| `token_class` | `input`, `output`, `cache_read`, `cache_write` or `reasoning` |
| `tokens` | the count, as text. Cached and cache-written tokens are taken out of `input`, reasoning tokens out of `output`: no token is counted twice |
| `request_id` | LiteLLM's call id |
| `cost_reported`, `cost_currency` | LiteLLM's computed cost of the call (USD) as plain decimal text, on the call's first event only |
| `payer` | `metadata.payer` of the request, when set |

The customer is the first identity found among `METEROID_CUSTOMER_ID_FROM` (default
`customer_org_alias,end_user,team_alias`; also `customer_member_id`, `team_id`, `user_id`,
`key_alias`), sent as the alias `ext:<identity>`. A call with none is not billed and logged.

## Prepaid credit gate

```bash
export METEROID_CREDIT_GATE=on
export METEROID_CREDIT_GATE_CURRENCY=EUR        # or METEROID_CREDIT_GATE_CREDIT_UNIT=<code>
```

Before each call the proxy asks Meteroid's wallet check (Meteroid Cloud and the Enterprise
images) whether the customer still has credit; a customer whose wallet pauses usage at zero and is
used up gets a `402`. The customer is resolved exactly as the call's events will name it. Answers
are cached for one second, and Meteroid's balance follows usage within a few seconds near zero, so
a burst can overspend slightly.

| Meteroid answers | The call |
|---|---|
| `200`, `has_access: true` | proceeds |
| `200`, `has_access: false` | is refused with `402` |
| nothing within `METEROID_CREDIT_GATE_TIMEOUT` seconds (default 1), or a network error | `METEROID_CREDIT_GATE_FAIL_OPEN` |
| `404` naming an unknown customer | is refused with `402` |
| `404` from an instance without the wallet check | `METEROID_CREDIT_GATE_FAIL_OPEN` |
| any other status | `METEROID_CREDIT_GATE_FAIL_OPEN` |

`METEROID_CREDIT_GATE_FAIL_OPEN` is `true` by default (the call proceeds); with `false` the call is
refused with `402`.

## Settings

| Variable | Default | |
|---|---|---|
| `METEROID_API_KEY` | required | the tenant's API key |
| `METEROID_API_BASE` | `https://api.meteroid.com` | |
| `METEROID_EVENT_CODE` | `llm_tokens` | |
| `METEROID_CUSTOMER_ID_FROM` | `customer_org_alias,end_user,team_alias` | ordered identity sources |
| `METEROID_BATCH_SIZE` | `100` | events per request, at most 500 |
| `METEROID_FLUSH_INTERVAL` | `5` | seconds |
| `METEROID_TIMEOUT` | `10` | seconds, for sending events |

Events older than 23 hours when sent (after an outage) are sent with `allow_backfilling`. A batch
Meteroid answers `401`, `403` (a key to fix), `429` or `5xx` to stays queued and is sent again;
one it refuses with another `4xx` is dropped and logged; events it rejects one by one (an unknown customer) are logged with the reason
and the others are kept. The queue holds 50,000 events, the oldest dropped first. The queue is
flushed every `METEROID_FLUSH_INTERVAL` seconds from the proxy's event loop and, for the SDK's
synchronous calls, from a background thread; what is left is sent when the process exits.

## Cost cards and the margin

Each cost card costs the events of its metric. Two cost cards on metrics that read the same
events (a total `llm_tokens` metric and a metric per token class, say) cost those events twice:
keep one cost card per event code.

## Development

```bash
python -m venv .venv && .venv/bin/pip install -e '.[test]'
.venv/bin/pytest                                   # unit tests, no network
cd tests/e2e && docker compose up -d && python3 run_e2e.py && docker compose down
```

`tests/fixtures/litellm-1.111.0-usage.json` holds the payloads LiteLLM 1.111.0 logs for Anthropic,
Bedrock and OpenAI usage (cache reads and writes, reasoning tokens), made by
`tests/fixtures/capture.py`; recapture them when upgrading LiteLLM.

The end-to-end run starts a real LiteLLM proxy with the callback, a mock provider and a mock of
Meteroid's ingest that refuses the first two batches: it checks that the batch sent three times
is billed once, that no prompt leaves the proxy, that an exhausted wallet is refused, and that
calls proceed while Meteroid is down.
