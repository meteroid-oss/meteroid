import { MessageInitShape } from '@bufbuild/protobuf'
import { Button } from '@md/ui'
import { Building2, ExternalLink, Key, LucideIcon, WebhookIcon } from 'lucide-react'
import { ReactNode } from 'react'
import { z } from 'zod'

import { CopyToClipboardButton } from '@/components/CopyToClipboard'
import { ConnectPaymentProviderRequestSchema } from '@/rpc/api/connectors/v1/connectors_pb'
import {
  ConnectorProviderEnum,
  PaymentProviderCapabilities,
} from '@/rpc/api/connectors/v1/models_pb'

import { GoCardlessLogo, MollieLogo, ProviderLogo, StancerLogo, StripeLogo } from './logos'

/**
 * Per-provider presentation only. Behaviour comes from the backend as
 * `PaymentProviderCapabilities`; adding a provider is one entry in `PAYMENT_PROVIDERS`.
 */

export type PaymentRail = 'card' | 'directDebit'

/** What the connect form knows about the tenant it runs in. */
export interface ConnectContext {
  tenantId?: string
  isProduction: boolean
  restApiUrl: string
  /** Live value of the alias field, for webhook URLs shown while typing. */
  alias: string
}

export interface ConnectField {
  label: string
  placeholder?: string | ((ctx: ConnectContext) => string)
  help?: string
  secret?: boolean
}

export interface ConnectStep {
  id: string
  title: string
  description: (ctx: ConnectContext) => ReactNode
  icon: LucideIcon
  /** Rendered inline as text inputs, in order. */
  fields: readonly string[]
  /** Provider-specific UI below the inputs (a disclosure, a derived read-only value). */
  extra?: (ctx: ConnectContext) => ReactNode
}

export type ProviderCredentials = NonNullable<
  MessageInitShape<typeof ConnectPaymentProviderRequestSchema>['credentials']
>

export interface ConnectDefinition {
  subtitle: string
  schema: z.ZodType<Record<string, string | undefined>>
  defaults: Record<string, string>
  steps: ConnectStep[]
  fields: Record<string, ConnectField>
  toCredentials: (
    values: Record<string, string | undefined>,
    ctx: ConnectContext
  ) => ProviderCredentials
  /** The backend registers our webhook endpoint itself; send it the URL to use. */
  autoRegisterWebhook?: boolean
  errorFallback: string
}

export interface PaymentProviderDefinition {
  provider: ConnectorProviderEnum
  /** Route segment and backend meta key. */
  key: string
  name: string
  description: string
  features: string[]
  Logo: ProviderLogo
  customerDashboardUrl?: (externalCustomerId: string, sandbox: boolean) => string
  /** Advisory check mirroring a provider-side requirement on the customer record. */
  customerAddressWarning?: (address: {
    hasAnyField: boolean
    missingCountry: boolean
  }) => string | undefined
  connect: ConnectDefinition
}

const aliasSchema = z
  .string()
  .min(1, 'Name is required')
  .regex(/^[a-z0-9-]+$/, 'Only lowercase letters, numbers, and hyphens allowed')

const aliasField = (example: string): ConnectField => ({
  label: 'Integration Name',
  help: `e.g., '${example}'`,
})

const webhookEndpointUrl = (ctx: ConnectContext) =>
  `${ctx.restApiUrl}/webhooks/v1/${ctx.tenantId}/${ctx.alias}`

const DashboardLink = ({ href, children }: { href: string; children: ReactNode }) => (
  <Button variant="link" hasIcon>
    <ExternalLink size={14} strokeWidth={1.5} />
    <a target="_blank" href={href} rel="noreferrer">
      {children}
    </a>
  </Button>
)

const stripe: PaymentProviderDefinition = {
  provider: ConnectorProviderEnum.STRIPE,
  key: 'stripe',
  name: 'Stripe',
  description: 'Global payments platform',
  features: ['Card', 'Direct Debit (SEPA, ACH, Bacs)', 'Link'],
  Logo: StripeLogo,
  customerDashboardUrl: id => `https://dashboard.stripe.com/customers/${id}`,
  connect: {
    subtitle: "Let's get your payments flowing in just a few steps",
    schema: z.object({
      alias: aliasSchema,
      apiPublishableKey: z
        .string()
        .min(1, 'Publishable key is required')
        .regex(/^pk_/, 'Should start with pk_'),
      // A standard secret key (`sk_`) or a restricted key (`rk_`) scoped for
      // webhook-endpoint creation.
      apiSecretKey: z
        .string()
        .min(1, 'Secret key is required')
        .regex(/^(sk|rk)_/, 'Should start with sk_ or rk_'),
      // Blank tells the backend to auto-create the endpoint via Stripe's API.
      webhookSecret: z
        .string()
        .optional()
        .refine(v => !v || /^whsec_/.test(v), 'Should start with whsec_'),
    }),
    defaults: { alias: 'stripe', apiPublishableKey: '', apiSecretKey: '', webhookSecret: '' },
    steps: [
      {
        id: 'alias',
        title: 'Connection',
        description: () => (
          <>
            Choose a unique alias to identify this connection.
            <br />
            You can connect multiple Stripe accounts.
          </>
        ),
        icon: Building2,
        fields: ['alias'],
      },
      {
        id: 'keys',
        title: 'API Keys',
        description: ctx => (
          <span>
            <span>
              Get your {!ctx.isProduction && 'test-mode'} API keys from your Stripe Dashboard
              under{' '}
            </span>
            <br />
            <DashboardLink
              href={`https://dashboard.stripe.com/${ctx.isProduction ? '' : 'test/'}apikeys`}
            >
              Developers → API keys
            </DashboardLink>
            <br />
            <span className="text-xs text-muted-foreground">
              We&apos;ll create the webhook endpoint for you. The API key needs the&nbsp;
              <code className="text-xs">Webhook Endpoints (write)</code> scope — if yours
              doesn&apos;t, expand <em>Advanced</em> to paste a signing secret manually.
            </span>
          </span>
        ),
        icon: Key,
        fields: ['apiPublishableKey', 'apiSecretKey'],
        extra: ctx => (
          <details className="rounded-md border border-border bg-card px-3 py-2">
            <summary className="cursor-pointer text-xs text-muted-foreground">
              Advanced: my API key can&apos;t create webhook endpoints
            </summary>
            <div className="pt-3 space-y-2">
              <p className="text-xs text-muted-foreground">
                Create a webhook in your Stripe dashboard pointing at this URL, then paste its
                signing secret here.
              </p>
              <CopyToClipboardButton text={webhookEndpointUrl(ctx)} className="whitespace-normal" />
              <ConnectFieldSlot name="webhookSecret" />
            </div>
          </details>
        ),
      },
    ],
    fields: {
      alias: aliasField('stripe-eu'),
      apiPublishableKey: {
        label: 'Publishable Key',
        placeholder: ctx => (ctx.isProduction ? 'pk_live_...' : 'pk_test_...'),
      },
      apiSecretKey: {
        label: 'Secret Key',
        placeholder: ctx => (ctx.isProduction ? 'sk_live_...' : 'sk_test_...'),
        secret: true,
      },
      webhookSecret: { label: 'Webhook Secret', placeholder: 'whsec_...', secret: true },
    },
    toCredentials: v => ({
      case: 'stripe',
      value: {
        alias: v.alias ?? '',
        apiPublishableKey: v.apiPublishableKey ?? '',
        apiSecretKey: v.apiSecretKey ?? '',
        webhookSecret: v.webhookSecret ?? '',
      },
    }),
    autoRegisterWebhook: true,
    errorFallback: 'Failed to connect. API key may be invalid.',
  },
}

const gocardless: PaymentProviderDefinition = {
  provider: ConnectorProviderEnum.GOCARDLESS,
  key: 'gocardless',
  name: 'GoCardless',
  description: 'Bank-debit collection across SEPA, BACS, ACH',
  features: ['Direct Debit (SEPA, BACS, ACH)', 'Recurring mandates'],
  Logo: GoCardlessLogo,
  customerDashboardUrl: (id, sandbox) =>
    `https://manage${sandbox ? '-sandbox' : ''}.gocardless.com/customers/${id}`,
  // GoCardless rejects customer/mandate creation with "country_code is required if any
  // address fields are provided".
  customerAddressWarning: ({ hasAnyField, missingCountry }) =>
    hasAnyField && missingCountry
      ? "Add the customer's country. GoCardless requires it once an address is set, otherwise setup fails."
      : undefined,
  connect: {
    subtitle: 'Set up bank-debit collection in a few steps',
    schema: z.object({
      alias: aliasSchema,
      accessToken: z.string().min(20, 'Access token looks too short'),
      webhookSecret: z.string().min(8, 'Webhook secret is required'),
      creditorId: z.string().optional(),
    }),
    defaults: { alias: 'gocardless', accessToken: '', webhookSecret: '', creditorId: '' },
    steps: [
      {
        id: 'alias',
        title: 'Connection',
        description: () => (
          <>
            Choose a unique alias to identify this connection.
            <br />
            You can connect multiple GoCardless accounts.
          </>
        ),
        icon: Building2,
        fields: ['alias'],
        // The environment follows the tenant (live in production, sandbox otherwise); not user-selectable.
        extra: ctx => (
          <div className="space-y-2">
            <label className="dark:text-muted-foreground font-normal text-xs">Environment</label>
            <div className="rounded-md border border-border bg-muted px-3 py-2 text-sm">
              {ctx.isProduction ? 'Live' : 'Sandbox'}
            </div>
            <p className="text-xs text-muted-foreground">
              Determined by this tenant — live for production, sandbox otherwise.
            </p>
          </div>
        ),
      },
      {
        id: 'keys',
        title: 'API Access',
        description: ctx => (
          <span>
            <span>Generate an access token in your GoCardless dashboard under </span>
            <br />
            <DashboardLink
              href={`https://manage${ctx.isProduction ? '' : '-sandbox'}.gocardless.com/developers/access-tokens`}
            >
              Developers → Access tokens
            </DashboardLink>
            <br />
            <span className="text-xs text-muted-foreground">
              The token needs read+write on customers, billing_requests, payments and mandates.
              Creditor id is optional — required only if your account has multiple creditors.
            </span>
          </span>
        ),
        icon: Key,
        fields: ['accessToken', 'creditorId'],
      },
      {
        id: 'webhook',
        title: 'Webhook Endpoint',
        description: ctx => (
          <span>
            <span>Create a webhook endpoint in your GoCardless dashboard under </span>
            <br />
            <DashboardLink
              href={`https://manage${ctx.isProduction ? '' : '-sandbox'}.gocardless.com/developers/webhook-endpoints`}
            >
              Developers → Webhook endpoints
            </DashboardLink>
            <br />
            <div className="bg-card p-4 rounded-lg space-y-3 mt-4">
              <ol className="space-y-2 text-sm text-card-foreground">
                <li>
                  Endpoint URL:
                  <br />
                  <CopyToClipboardButton
                    text={webhookEndpointUrl(ctx)}
                    buttonClassName="max-w-full h-auto items-start text-left whitespace-normal"
                    className="whitespace-normal break-all"
                  />
                </li>
                <li>
                  Subscribe to these event types: <br />
                  <div className="font-mono text-xs bg-background dark:bg-secondary rounded-md py-2">
                    <code>payments, mandates, billing_requests</code>
                  </div>
                  <br />
                </li>
                <li>Copy the signing secret into the form below.</li>
              </ol>
            </div>
          </span>
        ),
        icon: WebhookIcon,
        fields: ['webhookSecret'],
      },
    ],
    fields: {
      alias: aliasField('gocardless-uk'),
      accessToken: {
        label: 'Access Token',
        placeholder: ctx => (ctx.isProduction ? 'live_...' : 'sandbox_...'),
        secret: true,
      },
      creditorId: {
        label: 'Creditor ID',
        placeholder: 'CR000... (optional)',
        help: 'Only needed if your account has multiple creditors.',
      },
      webhookSecret: { label: 'Webhook Secret', secret: true },
    },
    toCredentials: (v, ctx) => ({
      case: 'gocardless',
      value: {
        alias: v.alias ?? '',
        accessToken: v.accessToken ?? '',
        webhookSecret: v.webhookSecret ?? '',
        creditorId: v.creditorId || undefined,
        environment: ctx.isProduction ? 'live' : 'sandbox',
      },
    }),
    errorFallback: 'Failed to connect. Access token may be invalid.',
  },
}

const stancer: PaymentProviderDefinition = {
  provider: ConnectorProviderEnum.STANCER,
  key: 'stancer',
  name: 'Stancer',
  description: 'European card payments platform',
  features: ['Card'],
  Logo: StancerLogo,
  connect: {
    subtitle: 'Set up card collection with your Stancer account',
    // The secret key is the only credential; its prefix selects test/live.
    schema: z.object({
      alias: aliasSchema,
      apiSecretKey: z
        .string()
        .min(1, 'Secret key is required')
        .regex(/^s(test|prod)_/, 'Should start with stest_ or sprod_'),
    }),
    defaults: { alias: 'stancer', apiSecretKey: '' },
    steps: [
      {
        id: 'keys',
        title: 'API Access',
        description: () => (
          <span>
            <span>Find your secret key in your Stancer dashboard under </span>
            <DashboardLink href="https://manage.stancer.com/en/developers">
              Developers → API keys
            </DashboardLink>
            <br />
            <span className="text-xs text-muted-foreground">
              A test key (stest_...) connects in test mode, a live key (sprod_...) in live mode.
            </span>
          </span>
        ),
        icon: Key,
        fields: ['alias', 'apiSecretKey'],
      },
    ],
    fields: {
      alias: aliasField('stancer-eu'),
      apiSecretKey: { label: 'Secret Key', placeholder: 'stest_...', secret: true },
    },
    toCredentials: v => ({
      case: 'stancer',
      value: { alias: v.alias ?? '', apiSecretKey: v.apiSecretKey ?? '' },
    }),
    errorFallback: 'Failed to connect. Secret key may be invalid.',
  },
}

const mollie: PaymentProviderDefinition = {
  provider: ConnectorProviderEnum.MOLLIE,
  key: 'mollie',
  name: 'Mollie',
  description: 'European payments platform',
  features: ['Card', 'Direct Debit (SEPA)', 'Recurring mandates'],
  Logo: MollieLogo,
  connect: {
    subtitle: 'Set up card and SEPA direct-debit collection with your Mollie account',
    // The API key is the only credential; its prefix selects test/live.
    schema: z.object({
      alias: aliasSchema,
      apiKey: z
        .string()
        .trim()
        .min(1, 'API key is required')
        .refine(
          v => !v.startsWith('access_'),
          "Access tokens aren't supported yet. Use a standard API key (test_… or live_…) from Developers → API keys."
        )
        .refine(
          v => v.startsWith('access_') || /^(test|live)_/.test(v),
          'Should start with test_ or live_'
        ),
    }),
    defaults: { alias: 'mollie', apiKey: '' },
    steps: [
      {
        id: 'keys',
        title: 'API Access',
        description: () => (
          <span>
            <span>Find your API key in your Mollie Dashboard under </span>
            <br />
            <DashboardLink href="https://my.mollie.com/dashboard/developers/api-keys">
              Developers → API keys
            </DashboardLink>
            <br />
            <span className="text-xs text-muted-foreground">
              Only standard API keys are supported. A test key (test_...) connects in test mode, a
              live key (live_...) in live mode. Enable cards and/or SEPA Direct Debit on the Mollie
              profile.
            </span>
          </span>
        ),
        icon: Key,
        fields: ['alias', 'apiKey'],
      },
    ],
    fields: {
      alias: aliasField('mollie-eu'),
      apiKey: { label: 'API Key', placeholder: 'test_...', secret: true },
    },
    toCredentials: v => ({
      case: 'mollie',
      value: { alias: v.alias ?? '', apiKey: v.apiKey ?? '' },
    }),
    errorFallback: 'Failed to connect. Please try again.',
  },
}

export const PAYMENT_PROVIDERS: PaymentProviderDefinition[] = [stripe, gocardless, stancer, mollie]

export const paymentProviderByKey = (key: string | undefined) =>
  PAYMENT_PROVIDERS.find(p => p.key === key)

export const paymentProvider = (provider: ConnectorProviderEnum | undefined) =>
  PAYMENT_PROVIDERS.find(p => p.provider === provider)

/** Display name for any connector provider, payment or not. */
export const connectorDisplayName = (provider: ConnectorProviderEnum | undefined): string => {
  switch (provider) {
    case ConnectorProviderEnum.HUBSPOT:
      return 'Hubspot'
    case ConnectorProviderEnum.PENNYLANE:
      return 'Pennylane'
    default:
      return paymentProvider(provider)?.name ?? 'Unknown'
  }
}

export const railsOf = (
  caps: PaymentProviderCapabilities | undefined
): Record<PaymentRail, boolean> => ({
  card: caps?.supportsCards ?? true,
  directDebit: caps?.supportsDirectDebit ?? true,
})

/**
 * Placeholder for a field inside a step's `extra` block; the connect modal swaps in the bound
 * input, keeping form plumbing out of provider definitions.
 */
export const ConnectFieldSlot = ({ name }: { name: string }) => <span data-connect-field={name} />
