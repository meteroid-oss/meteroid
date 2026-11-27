/**
 * The Launch pad's tracks and steps. A step is done when its `signal` shows up in the tenant's
 * data (see useGuideProgress); steps without one count as done once opened from the guide.
 */

/** Data the guide can see, each one "is there at least one?". */
export type GuideSignal =
  | 'plans'
  | 'customers'
  | 'subscriptions'
  | 'invoices'
  | 'quotes'
  | 'acceptedQuotes'
  | 'metrics'
  | 'events'

export interface GuideStep {
  id: string
  title: string
  description: string
  cta: {
    label: string
    /** Path under the tenant (`''` is Home). */
    to?: string
    /** Opens the create-customer panel in place instead of navigating. */
    action?: 'createCustomer'
  }
  /** Needed to take the step; without it the guide points to an admin. */
  permission?: string
  signal?: GuideSignal
}

export interface GuideTrack {
  id: 'billing' | 'cpq' | 'metering'
  label: string
  steps: GuideStep[]
}

export const GUIDE_TRACKS: GuideTrack[] = [
  {
    id: 'billing',
    label: 'Billing',
    steps: [
      {
        id: 'billing.plan',
        title: 'Create your first plan',
        description:
          'Decide what you sell and how it is priced: a flat fee, per seat or by usage. Plans are versioned, so you can change prices later without touching existing customers.',
        cta: { label: 'Create a plan', to: 'plans/add-plan' },
        permission: 'catalog:manage',
        signal: 'plans',
      },
      {
        id: 'billing.customer',
        title: 'Add a customer',
        description:
          'A customer holds the billing details: currency, address, and which of your invoicing entities bills them.',
        cta: { label: 'Add a customer', action: 'createCustomer' },
        permission: 'customers:manage',
        signal: 'customers',
      },
      {
        id: 'billing.subscription',
        title: 'Start a subscription',
        description:
          'Put a customer on a plan. From then on Meteroid bills every period by itself, prorating when the plan changes.',
        cta: { label: 'Start a subscription', to: 'subscriptions/create' },
        permission: 'billing:subscriptions:manage',
        signal: 'subscriptions',
      },
      {
        id: 'billing.invoice',
        title: 'See automatic invoicing in action',
        description:
          'Each billing period produces an invoice: drafted, finalized and sent to the customer, with its payment tracked.',
        cta: { label: 'Open invoices', to: 'invoices' },
        permission: 'billing:invoices:read',
        signal: 'invoices',
      },
      {
        id: 'billing.revenue',
        title: 'View revenue reporting',
        description:
          'Home charts revenue and MRR over time, with the new business, expansions and churn behind every change.',
        cta: { label: 'Open Home', to: '' },
      },
    ],
  },
  {
    id: 'cpq',
    label: 'CPQ',
    steps: [
      {
        id: 'cpq.quote',
        title: 'Build a quote',
        description:
          'Pick a customer and a plan, adjust prices, discounts and terms, and preview exactly what they will sign.',
        cta: { label: 'Build a quote', to: 'quotes/create' },
        permission: 'billing:quotes:manage',
        signal: 'quotes',
      },
      {
        id: 'cpq.accepted',
        title: 'Get a quote accepted',
        description:
          'Send the quote for signature. Your customer reviews and accepts it online, and every step is recorded.',
        cta: { label: 'Open quotes', to: 'quotes' },
        permission: 'billing:quotes:read',
        signal: 'acceptedQuotes',
      },
      {
        id: 'cpq.convert',
        title: 'Turn an accepted quote into a subscription',
        description:
          'Convert an accepted quote into a subscription with the agreed prices, so billing starts without retyping anything.',
        cta: { label: 'Open quotes', to: 'quotes' },
        permission: 'billing:quotes:manage',
      },
    ],
  },
  {
    id: 'metering',
    label: 'Metering',
    steps: [
      {
        id: 'metering.metric',
        title: 'Define a billable metric',
        description:
          'Say what to count, such as API calls, seats or gigabytes stored, and how events add up over a period.',
        cta: { label: 'Create a metric', to: 'metrics/add-metric' },
        permission: 'catalog:manage',
        signal: 'metrics',
      },
      {
        id: 'metering.events',
        title: 'Send usage events',
        description:
          'Report usage from your application through the API. Create an API key, then send an event for each billable action.',
        cta: { label: 'Create an API key', to: 'settings/api-keys' },
        signal: 'events',
      },
      {
        id: 'metering.price',
        title: 'Price usage in a plan',
        description:
          'Add a usage-based component to a plan, so the events your customers generate turn into charges on their invoices.',
        cta: { label: 'Open plans', to: 'plans' },
        permission: 'catalog:manage',
      },
    ],
  },
]
