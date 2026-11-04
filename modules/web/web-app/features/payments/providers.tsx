import { MessageInitShape } from '@bufbuild/protobuf'
import { Button } from '@md/ui'
import { ExternalLink } from 'lucide-react'
import { ReactNode } from 'react'
import { z } from 'zod'

import { UpdatePaymentProviderCredentialsRequestSchema } from '@/rpc/api/connectors/v1/connectors_pb'
import {
  ConnectorProviderEnum,
  PaymentProviderCapabilities,
} from '@/rpc/api/connectors/v1/models_pb'

import {
  GoCardlessLogo,
  MockLogo,
  MollieLogo,
  ProviderLogo,
  StancerLogo,
  StripeLogo,
} from './logos'

/**
 * Per-provider presentation only. Behaviour comes from the backend as
 * `PaymentProviderCapabilities`; adding a provider is one entry in `PAYMENT_PROVIDERS`.
 */

export type PaymentRail = 'card' | 'directDebit'

/** What the connect forms know about the tenant and the connection they run for. */
export interface ConnectContext {
  isProduction: boolean
  /** Server-built; known once the draft exists. */
  webhookUrl?: string
}

export interface ConnectField {
  label: string
  placeholder?: string | ((ctx: ConnectContext) => string)
  help?: string
  secret?: boolean
  /** Validates a non-blank value. */
  rule: z.ZodType<string>
  /** May be left blank when first connecting. */
  optional?: boolean
  /** The secret `ProviderAccount.keyHint` masks; shown as the current value when rotating. */
  keyHint?: boolean
}

export interface ConnectWebhook {
  /** `auto`: the backend registers the endpoint itself (the secret is a fallback). */
  mode: 'auto' | 'manual'
  /** How to create the endpoint by hand, around the copyable URL. */
  instructions: (ctx: ConnectContext) => ReactNode
  /** Field holding the signing secret, from `fields`. */
  secretField: string
}

export type ProviderCredentials = NonNullable<
  MessageInitShape<typeof UpdatePaymentProviderCredentialsRequestSchema>['credentials']
>

export interface ConnectDefinition {
  subtitle: string
  /** Prefill and example for the alias chosen in step 1. */
  aliasDefault: string
  aliasExample: string
  /** Step 1 shows the environment, which follows the tenant. */
  showsEnvironment?: boolean
  /** Explains where to find the credentials. */
  credentialsHelp: (ctx: ConnectContext) => ReactNode
  /** Credential inputs, in order. */
  credentialFields: readonly string[]
  /** Absent: the provider needs no webhook setup, so the wizard skips that step. */
  webhook?: ConnectWebhook
  fields: Record<string, ConnectField>
  /** Blank values are sent as empty, which the backend reads as "keep the stored value". */
  toCredentials: (
    values: Record<string, string | undefined>,
    ctx: ConnectContext
  ) => ProviderCredentials
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

export const aliasSchema = z
  .string()
  .min(1, 'Name is required')
  .regex(/^[a-z0-9-]+$/, 'Only lowercase letters, numbers, and hyphens allowed')

/**
 * Schema for `fields`. `create` requires every non-optional field; `update` accepts blanks, which
 * keep the stored value.
 */
export const credentialsSchema = (
  connect: ConnectDefinition,
  fields: readonly string[],
  mode: 'create' | 'update'
) =>
  z.object(
    Object.fromEntries(
      fields.map(name => {
        const field = connect.fields[name]
        return [
          name,
          mode === 'create' && !field.optional
            ? field.rule
            : z.union([z.literal(''), field.rule]).optional(),
        ]
      })
    )
  ) as z.ZodType<Record<string, string | undefined>>

const DashboardLink = ({ href, children }: { href: string; children: ReactNode }) => (
  <Button variant="link" hasIcon className="h-fit p-0">
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
    aliasDefault: 'stripe',
    aliasExample: 'stripe-eu',
    credentialsHelp: ctx => (
      <>
        Get your {!ctx.isProduction && 'test-mode'} API keys from your Stripe Dashboard under{' '}
        <DashboardLink
          href={`https://dashboard.stripe.com/${ctx.isProduction ? '' : 'test/'}apikeys`}
        >
          Developers → API keys
        </DashboardLink>
        <br />
        <span className="text-xs">
          We&apos;ll create the webhook endpoint for you, which needs the{' '}
          <code className="text-xs">Webhook Endpoints (write)</code> scope on restricted keys.
        </span>
      </>
    ),
    credentialFields: ['apiPublishableKey', 'apiSecretKey'],
    webhook: {
      mode: 'auto',
      instructions: () => (
        <>
          Create a webhook endpoint in your Stripe dashboard with the URL below, subscribed to
          payment, mandate, refund and dispute events, then paste its signing secret.
        </>
      ),
      secretField: 'webhookSecret',
    },
    fields: {
      apiPublishableKey: {
        label: 'Publishable Key',
        placeholder: ctx => (ctx.isProduction ? 'pk_live_...' : 'pk_test_...'),
        rule: z.string().min(1, 'Publishable key is required').regex(/^pk_/, 'Should start with pk_'),
      },
      // A standard secret key (`sk_`) or a restricted key (`rk_`) scoped for
      // webhook-endpoint creation.
      apiSecretKey: {
        label: 'Secret Key',
        placeholder: ctx => (ctx.isProduction ? 'sk_live_...' : 'sk_test_...'),
        secret: true,
        keyHint: true,
        rule: z
          .string()
          .min(1, 'Secret key is required')
          .regex(/^(sk|rk)_/, 'Should start with sk_ or rk_'),
      },
      webhookSecret: {
        label: 'Webhook Secret',
        placeholder: 'whsec_...',
        secret: true,
        optional: true,
        rule: z.string().regex(/^whsec_/, 'Should start with whsec_'),
      },
    },
    toCredentials: v => ({
      case: 'stripe',
      value: {
        apiPublishableKey: v.apiPublishableKey ?? '',
        apiSecretKey: v.apiSecretKey ?? '',
        webhookSecret: v.webhookSecret ?? '',
      },
    }),
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
    aliasDefault: 'gocardless',
    aliasExample: 'gocardless-uk',
    showsEnvironment: true,
    credentialsHelp: ctx => (
      <>
        Generate an access token in your GoCardless dashboard under{' '}
        <DashboardLink
          href={`https://manage${ctx.isProduction ? '' : '-sandbox'}.gocardless.com/developers/access-tokens`}
        >
          Developers → Access tokens
        </DashboardLink>
        <br />
        <span className="text-xs">
          The token needs read+write on customers, billing_requests, payments and mandates.
        </span>
      </>
    ),
    credentialFields: ['accessToken', 'creditorId'],
    webhook: {
      mode: 'manual',
      instructions: ctx => (
        <>
          In GoCardless →{' '}
          <DashboardLink
            href={`https://manage${ctx.isProduction ? '' : '-sandbox'}.gocardless.com/developers/webhook-endpoints`}
          >
            Developers → Webhook endpoints
          </DashboardLink>
          , create an endpoint with this URL, subscribed to{' '}
          <code className="text-xs">payments, mandates, billing_requests</code>:
        </>
      ),
      secretField: 'webhookSecret',
    },
    fields: {
      accessToken: {
        label: 'Access Token',
        placeholder: ctx => (ctx.isProduction ? 'live_...' : 'sandbox_...'),
        secret: true,
        keyHint: true,
        rule: z.string().min(20, 'Access token looks too short'),
      },
      creditorId: {
        label: 'Creditor ID (optional)',
        placeholder: 'CR000...',
        help: 'Only needed if your account has multiple creditors.',
        optional: true,
        rule: z.string(),
      },
      webhookSecret: {
        label: 'Webhook Secret',
        secret: true,
        rule: z.string().min(8, 'Webhook secret is required'),
      },
    },
    toCredentials: (v, ctx) => ({
      case: 'gocardless',
      value: {
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
    aliasDefault: 'stancer',
    aliasExample: 'stancer-eu',
    credentialsHelp: () => (
      <>
        Find your secret key in your Stancer dashboard under{' '}
        <DashboardLink href="https://manage.stancer.com/en/developers">
          Developers → API keys
        </DashboardLink>
        <br />
        <span className="text-xs">
          A test key (stest_...) connects in test mode, a live key (sprod_...) in live mode.
        </span>
      </>
    ),
    // The secret key is the only credential; its prefix selects test/live.
    credentialFields: ['apiSecretKey'],
    fields: {
      apiSecretKey: {
        label: 'Secret Key',
        placeholder: 'stest_...',
        secret: true,
        keyHint: true,
        rule: z
          .string()
          .min(1, 'Secret key is required')
          .regex(/^s(test|prod)_/, 'Should start with stest_ or sprod_'),
      },
    },
    toCredentials: v => ({
      case: 'stancer',
      value: { apiSecretKey: v.apiSecretKey ?? '' },
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
    aliasDefault: 'mollie',
    aliasExample: 'mollie-eu',
    credentialsHelp: () => (
      <>
        Find your Standard API key in your Mollie Dashboard under{' '}
        <DashboardLink href="https://my.mollie.com/dashboard/developers/api-keys">
          Developers → API keys
        </DashboardLink>
        <br />
        <span className="text-xs">
          A test key (test_...) connects in test mode, a live key (live_...) in live mode. Enable
          cards and/or SEPA Direct Debit on the Mollie profile.
        </span>
      </>
    ),
    // The API key is the only credential; its prefix selects test/live. Webhooks are per payment.
    credentialFields: ['apiKey'],
    fields: {
      apiKey: {
        label: 'Standard API Key',
        placeholder: 'test_...',
        secret: true,
        keyHint: true,
        rule: z
          .string()
          .trim()
          .min(1, 'Standard API key is required')
          .refine(
            v => !v.startsWith('access_'),
            "Access tokens aren't supported yet. Use a standard API key (test_… or live_…) from Developers → API keys."
          )
          .refine(
            v => v.startsWith('access_') || /^(test|live)_/.test(v),
            'Should start with test_ or live_'
          ),
      },
    },
    toCredentials: v => ({
      case: 'mollie',
      value: { apiKey: v.apiKey ?? '' },
    }),
    errorFallback: 'Failed to connect. Please try again.',
  },
}

/**
 * Simulated provider. No credentials and no instrument data: the customer picks a scripted
 * scenario at checkout. The backend refuses it unless the instance enables it and the tenant is
 * not production; the settings page lists it under the same conditions.
 */
const mock: PaymentProviderDefinition = {
  provider: ConnectorProviderEnum.MOCK,
  key: 'mock',
  name: 'Test gateway',
  description: 'Try checkouts and payments end to end without a payment account',
  features: [
    'Card',
    'Direct Debit (SEPA, ACH, Bacs)',
    'Scripted outcomes: success, decline, 3DS, pending',
  ],
  Logo: MockLogo,
  connect: {
    subtitle: 'Nothing to configure: name the connection and connect',
    aliasDefault: 'test-gateway',
    aliasExample: 'test-gateway',
    credentialsHelp: () => (
      <>
        The test gateway needs no credentials. Verifying detects its simulated account.
        <br />
        <span className="text-xs">
          No money moves and no card or bank details are ever collected: at checkout, a test
          scenario is picked instead. Pending payments are settled from the invoice page.
        </span>
      </>
    ),
    credentialFields: [],
    fields: {},
    toCredentials: () => ({ case: 'mock', value: { alias: '' } }),
    errorFallback: 'Failed to connect the test gateway.',
  },
}

/** Offered to every tenant. The mock is listed separately, only where the instance enables it. */
export const PAYMENT_PROVIDERS: PaymentProviderDefinition[] = [stripe, gocardless, stancer, mollie]
export const MOCK_PAYMENT_PROVIDER = mock

const ALL_PAYMENT_PROVIDERS = [...PAYMENT_PROVIDERS, mock]

export const paymentProviderByKey = (key: string | undefined) =>
  ALL_PAYMENT_PROVIDERS.find(p => p.key === key)

export const paymentProvider = (provider: ConnectorProviderEnum | undefined) =>
  ALL_PAYMENT_PROVIDERS.find(p => p.provider === provider)

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
