# @md/portal-embed

Embed the Meteroid customer billing portal in your own app — either the full
self-service portal or a compact widget (`plan`, `usage`, `invoices`,
`payment-methods`). Ships a dependency-free vanilla API and an optional React
wrapper.

- **Runtime-dependency-free.** React is an _optional_ peer; the vanilla build
  never imports it.
- **Auto-resizing iframe.** The embed reports its height; the host grows the
  iframe so it never scrolls or clips.
- **Themeable.** Forward `theme` / `accent` / `radius` overrides straight onto
  the portal.

## Install

```sh
pnpm add @md/portal-embed
# react is only needed for the React entry point
pnpm add react react-dom
```

## Quick start — vanilla (`<script>`)

```html
<div id="billing"></div>
<script type="module">
  import { mountBillingPortal } from '@md/portal-embed'

  const handle = mountBillingPortal('#billing', {
    token: PORTAL_TOKEN, // minted server-side, see below
    baseUrl: 'https://app.meteroid.com',
    view: 'plan',
    theme: 'light',
    accent: '#C6F94E',
    radius: 'Modern',
    onNavigate: target => {
      // Optional. Provide this to handle widget buttons ("Manage", "View all", …)
      // yourself — `target` is a portal page (`portal`, `subscriptions`, `usage`,
      // `invoices`, `settings`). Omit it and the widget opens the full portal in
      // a new tab instead.
      location.href = `/billing?page=${target}`
    },
  })

  // later: handle.destroy()
</script>
```

## Quick start — React

```tsx
import { BillingPortal, BillingEmbed } from '@md/portal-embed/react'

export function Billing({ token }: { token: string }) {
  return (
    <>
      {/* full portal */}
      <BillingPortal token={token} accent="#C6F94E" />

      {/* compact widget — BillingEmbed takes `view` up front */}
      <BillingEmbed view="invoices" token={token} theme="dark" />
    </>
  )
}
```

## Minting a portal token (proxy pattern)

The `token` is a **customer-scoped, time-limited JWT** (24h by default; override
with `expires_in_seconds`, range `60..2592000`). Never ship your tenant API key to
the browser — mint the token on your backend with your API key and hand only the
resulting token to the front end. There is no in-browser refresh, so mint a fresh
token per session.

```js
// Node / Express — your backend, authenticated as *your* user.
import express from 'express'

const app = express()

app.post('/api/billing-token', async (req, res) => {
  const customerId = req.user.meteroidCustomerId // your mapping

  const r = await fetch(
    `https://app.meteroid.com/api/v1/customers/${customerId}/portal-token`,
    {
      method: 'POST',
      headers: {
        Authorization: `Bearer ${process.env.METEROID_API_KEY}`,
        'Content-Type': 'application/json',
      },
      // Optional body — omit for the 24h default.
      body: JSON.stringify({ expires_in_seconds: 3600 }),
    }
  )

  const { token, portal_url } = await r.json()
  res.json({ token, portalUrl: portal_url })
})
```

The browser then fetches `/api/billing-token` and passes `token` to
`mountBillingPortal` / `<BillingPortal>`.

## Views (`view` / `?embed=`)

| `view`             | Renders                                            |
| ------------------ | -------------------------------------------------- |
| `portal`           | The full self-service portal (default)             |
| `plan`             | Compact current-plan summary card                  |
| `subscriptions`    | Compact list of active subscriptions               |
| `subscription`     | Single subscription summary (needs `subscriptionId`)|
| `usage`            | Metered usage this period, per subscription + charts|
| `invoices`         | Compact, paginated recent-invoices list            |
| `payment-methods`  | Compact payment-methods list (set-default / add)   |

The `invoices` widget takes a `count` option (rows per page, default `5`,
clamped to `1..20`) — e.g. `<BillingEmbed view="invoices" count={10} … />`.

The `subscription` widget needs a `subscriptionId` (maps to `?subscription=`) —
e.g. `<BillingEmbed view="subscription" subscriptionId="sub_123" … />`.

All widgets render on a **transparent** background so they blend into the host
page; only the cards carry a surface.

## Branding

Every widget shows a small **Powered by Meteroid** attribution by default. Hide
it with `branding={false}` (vanilla: `branding: false`, script tag:
`data-branding="false"`), which maps to `?branding=false` on the iframe. The
tenant's portal branding settings can also disable it globally.

## Theme overrides

These map onto the portal's URL override params. Anything omitted falls back to
the tenant's branding, then the built-in defaults.

| Option   | Values                          | Maps to URL param |
| -------- | ------------------------------- | ----------------- |
| `theme`  | `light` \| `dark`               | `?theme=`         |
| `accent` | hex color, e.g. `#C6F94E`       | `?accent=`        |
| `radius` | `Sharp` \| `Modern` \| `Rounded`| `?radius=`        |

### Curated palette overrides

To match a host product's look beyond the accent, override a small, stable set
of colors (hex). Anything omitted stays derived from `accent` / `theme`. These
are intentionally limited — the full internal palette is not a public contract.

| Option    | Sets                  | Maps to URL param |
| --------- | --------------------- | ----------------- |
| `bg`      | page background       | `?bg=`            |
| `surface` | card / panel surface  | `?surface=`       |
| `text`    | primary text color    | `?text=`          |
| `border`  | border / divider      | `?border=`        |

```tsx
<BillingEmbed view="invoices" surface="#FFFFFF" text="#0B0D12" border="#E2E5EA" />
```

## postMessage protocol

The embedded portal posts these messages to the host window; the SDK validates
`event.origin` against the iframe's origin before acting on them.

| Message                                        | Effect                                  |
| ---------------------------------------------- | --------------------------------------- |
| `{ type: 'meteroid:resize', height }`          | SDK sets the iframe height              |
| `{ type: 'meteroid:navigate', target }`        | SDK calls `onNavigate(target)`          |

`meteroid:navigate` is only posted when you pass `onNavigate` (the SDK then adds
`nav=host` to the iframe URL). Without it, the widget opens the full portal in a
new tab itself and posts nothing.

## API

- `mountBillingPortal(target, options) → { destroy() }` — `target` is a CSS
  selector or an `HTMLElement`.
- `buildEmbedUrl(options) → string` — the iframe `src`, if you want to render the
  iframe yourself.
- `<BillingPortal {...options} />` and `<BillingEmbed view {...options} />` from
  `@md/portal-embed/react`.

See `BillingPortalOptions` for the full option list.
