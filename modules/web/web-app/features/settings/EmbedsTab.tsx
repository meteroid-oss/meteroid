import { FunctionComponent } from 'react'

/**
 * Developer-facing documentation for embedding the customer billing portal
 * into the tenant's own product. The portal itself is at
 * `/portal/customer?token=<CUSTOMER_PORTAL_TOKEN>`; tokens are minted
 * server-side and the `@md/portal-embed` SDK wraps the iframe.
 */
const Code = ({ children }: { children: string }) => (
  <pre className="bg-muted text-foreground rounded-lg p-4 text-xs font-mono overflow-x-auto leading-relaxed">
    {children}
  </pre>
)

const Section = ({
  title,
  description,
  children,
}: {
  title: string
  description?: string
  children: React.ReactNode
}) => (
  <div className="border border-border rounded-lg p-6 space-y-3 max-w-3xl">
    <div>
      <h2 className="text-base font-semibold">{title}</h2>
      {description && <p className="text-sm text-muted-foreground mt-1">{description}</p>}
    </div>
    {children}
  </div>
)

export const EmbedsTab: FunctionComponent = () => {
  return (
    <div className="py-4 space-y-5">
      <div>
        <h1 className="text-lg font-semibold">Customer portal embeds</h1>
        <p className="text-sm text-muted-foreground max-w-2xl">
          Embed the customer billing portal — or individual widgets — directly inside your product.
          Widgets inherit your portal branding and stay in sync with each customer&apos;s live
          billing state. Branding defaults are configured under Settings → Invoice → Customer
          portal.
        </p>
      </div>

      <Section
        title="1. Mint a portal token (server-side)"
        description="Generate a short-lived, customer-scoped token from your backend using your API key. Never expose your API key to the browser."
      >
        <Code>{`curl -X POST https://api.meteroid.com/api/v1/customers/{customer_id}/portal-token \\
  -H "Authorization: Bearer $METEROID_API_KEY"
# => { "token": "...", "portal_url": "https://app.meteroid.com/portal/customer?token=..." }`}</Code>
      </Section>

      <Section
        title="2a. Drop-in script tag"
        description="The loader is served same-origin at /embed.js. Add the data attributes and it auto-mounts an auto-resizing iframe on load — no JS required."
      >
        <Code>{`<script
  src="https://<your-app-host>/embed.js"
  data-meteroid-portal
  data-token="CUSTOMER_PORTAL_TOKEN"
  data-view="invoices"
  data-theme="dark"
  data-accent="#C6F94E"
></script>`}</Code>
      </Section>

      <Section
        title="2b. Programmatic mount"
        description="Prefer to control where it mounts? The same script exposes window.Meteroid."
      >
        <Code>{`<script src="https://<your-app-host>/embed.js"></script>
<div id="billing"></div>
<script>
  Meteroid.mountBillingPortal('#billing', {
    token: 'CUSTOMER_PORTAL_TOKEN',
    view: 'portal',      // 'portal' | 'plan' | 'usage' | 'invoices' | 'payment-methods'
    theme: 'dark',       // optional override
    accent: '#C6F94E',   // optional override
  })
</script>`}</Code>
      </Section>

      <Section
        title="2c. React component"
        description="Install @md/portal-embed and render the typed wrapper."
      >
        <Code>{`import { BillingPortal } from '@md/portal-embed/react'

<BillingPortal token={portalToken} view="plan" accent="#C6F94E" radius="Modern" />`}</Code>
      </Section>

      <Section
        title="Override parameters"
        description="Any of these can be passed to override the tenant's configured portal defaults."
      >
        <div className="text-sm">
          <table className="w-full text-left">
            <thead className="text-muted-foreground">
              <tr>
                <th className="py-1.5 pr-6 font-medium">Param</th>
                <th className="py-1.5 font-medium">Values</th>
              </tr>
            </thead>
            <tbody className="font-mono text-xs">
              <tr className="border-t border-border">
                <td className="py-1.5 pr-6">view</td>
                <td className="py-1.5">
                  portal · plan · subscriptions · subscription · usage · invoices · payment-methods
                </td>
              </tr>
              <tr className="border-t border-border">
                <td className="py-1.5 pr-6">subscription</td>
                <td className="py-1.5">subscription id (for the subscription view)</td>
              </tr>
              <tr className="border-t border-border">
                <td className="py-1.5 pr-6">theme</td>
                <td className="py-1.5">light · dark</td>
              </tr>
              <tr className="border-t border-border">
                <td className="py-1.5 pr-6">accent</td>
                <td className="py-1.5">#RRGGBB</td>
              </tr>
              <tr className="border-t border-border">
                <td className="py-1.5 pr-6">radius</td>
                <td className="py-1.5">Sharp · Modern · Rounded</td>
              </tr>
              <tr className="border-t border-border">
                <td className="py-1.5 pr-6">bg · surface · text · border</td>
                <td className="py-1.5">#RRGGBB (match the host palette)</td>
              </tr>
            </tbody>
          </table>
        </div>
      </Section>
    </div>
  )
}
