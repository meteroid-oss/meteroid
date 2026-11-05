import { Badge, Button, Card } from '@md/ui'
import { ArrowLeftRightIcon, ExternalLinkIcon, GiftIcon, LucideIcon } from 'lucide-react'
import { FunctionComponent } from 'react'

import {
  SettingsRow,
  SettingsRowGroup,
  SettingsSection,
} from '@/features/settings/components/SettingsSection'

const PRICING_URL = 'https://meteroid.com/pricing'
const SWITCH_EDITIONS_DOCS_URL = 'https://docs.meteroid.com/self-hosting/editions'
const LICENSE_URL = 'https://github.com/meteroid-oss/meteroid/blob/main/editions/enterprise/LICENSE'

type Availability = 'free' | 'licensed'

interface EnterpriseFeature {
  name: string
  description: string
  availability: Availability
}

interface EnterpriseArea {
  area: string
  summary?: string
  features: EnterpriseFeature[]
}

const HIGHLIGHTS: { icon: LucideIcon; title: string; description: string }[] = [
  {
    icon: GiftIcon,
    title: 'Free for small companies',
    description:
      'The Enterprise images are free under 50 employees and €2M in annual revenue. A license unlocks the paid features.',
  },
  {
    icon: ArrowLeftRightIcon,
    title: 'Switch both ways',
    description:
      'Same database and migrations: change the images, and change them back the same way.',
  },
]

const ENTERPRISE_FEATURES: EnterpriseArea[] = [
  {
    area: 'Billing & invoicing',
    features: [
      {
        name: 'Dunning',
        description: 'Automated payment reminder sequences for overdue invoices.',
        availability: 'free',
      },
      {
        name: 'Tax reports',
        description: 'VAT return and EC Sales List, ready to file.',
        availability: 'free',
      },
      {
        name: 'Quotes with several signers',
        description: 'Collect signatures from several people on a quote.',
        availability: 'licensed',
      },
      {
        name: 'Purchase orders on quotes',
        description: 'Let customers add their purchase order to the quote they sign.',
        availability: 'licensed',
      },
      {
        name: 'Customer email customization',
        description:
          'Choose which emails go out and to whom, set the sender, preview and send tests, and see what was sent.',
        availability: 'free',
      },
      {
        name: 'E-invoicing',
        description: 'Issue Factur-X / ZUGFeRD and XRechnung e-invoices alongside the PDF.',
        availability: 'free',
      },
      {
        name: 'Invoice consolidation',
        description: "Bill all of a customer's subscriptions on a single invoice.",
        availability: 'licensed',
      },
      {
        name: 'Minimum commitments',
        description:
          'Charge a minimum per billing period on plans, subscriptions and quotes, with the shortfall invoiced automatically.',
        availability: 'licensed',
      },
      {
        name: 'Multiple invoicing entities',
        description:
          'Invoice from several legal entities, each with its own details and numbering.',
        availability: 'licensed',
      },
      {
        name: 'Invoice annexes',
        description: 'Attach detailed usage tables, built from your events, to invoices.',
        availability: 'licensed',
      },
      {
        name: 'Custom properties',
        description:
          'Add your own typed, validated fields to customers, subscriptions and invoices.',
        availability: 'licensed',
      },
    ],
  },
  {
    area: 'Intelligence',
    features: [
      {
        name: 'MRR & revenue reports',
        description: 'Track recurring revenue, revenue and their movements over time.',
        availability: 'free',
      },
      {
        name: 'Advanced analytics',
        description: 'Explore and break down any metric across customers, plans and segments.',
        availability: 'licensed',
      },
      {
        name: 'Dashboards',
        description: 'Build and share dashboards of the metrics your team follows.',
        availability: 'licensed',
      },
      {
        name: 'Customer signals',
        description: 'See which customers are at risk and which are ready to expand.',
        availability: 'licensed',
      },
      {
        name: 'Steering',
        description: 'Set targets on your metrics and get alerted when they drift.',
        availability: 'licensed',
      },
    ],
  },
  {
    area: 'Credits',
    summary:
      'Prepaid credit in money is in the open-source edition; the rest comes with the images.',
    features: [
      {
        name: 'Credit units',
        description: 'Price and grant usage in your own credits instead of money.',
        availability: 'free',
      },
      {
        name: 'Credit packs',
        description: 'Include credit in plans, add-ons and subscriptions, or sell it as packs.',
        availability: 'free',
      },
      {
        name: 'Self-serve top-ups',
        description: 'Let customers buy credit from the portal.',
        availability: 'free',
      },
      {
        name: 'Scoped credit',
        description: 'Grant credit for one subscription, usage only or some products.',
        availability: 'licensed',
      },
    ],
  },
  {
    area: 'Customer portal',
    summary: 'The open-source portal shows everything and lets customers pay and cancel.',
    features: [
      {
        name: 'Self-service',
        description:
          'Customers change plan, add-ons and seats, top up credit and redeem codes themselves.',
        availability: 'free',
      },
      {
        name: 'Embedded portal',
        description: 'Show the portal and its widgets inside your own application.',
        availability: 'licensed',
      },
      {
        name: 'Portal theming',
        description: 'Light or dark theme and corner roundness, on top of your logo and colors.',
        availability: 'licensed',
      },
    ],
  },
  {
    area: 'Usage controls',
    features: [
      {
        name: 'Live balances & usage limits',
        description:
          'Live credit balances, low-balance alerts, hard limits and the real-time access check, and what happens when credit runs out.',
        availability: 'free',
      },
      {
        name: 'Usage costs and running totals',
        description:
          "Read what each customer's usage costs over time, and what the next invoice bills so far.",
        availability: 'licensed',
      },
      {
        name: 'Budgets',
        description: 'Spending budgets with alerts, per customer.',
        availability: 'licensed',
      },
      {
        name: 'Automatic top-ups',
        description: 'Buy credit automatically when a wallet runs low.',
        availability: 'licensed',
      },
    ],
  },
  {
    area: 'Security & access',
    features: [
      {
        name: 'Single sign-on',
        description:
          'Sign your team in through your identity provider (OIDC) on the domains you verify, and enforce it.',
        availability: 'licensed',
      },
      {
        name: 'Enforced two-factor authentication',
        description: 'Require two-factor authentication for every member of your organization.',
        availability: 'licensed',
      },
      {
        name: 'Custom roles',
        description: 'Define roles as sets of permissions and assign them to members.',
        availability: 'licensed',
      },
      {
        name: 'Security log & audit log',
        description: 'Sign-ins and membership changes, and every change across the workspace.',
        availability: 'licensed',
      },
    ],
  },
  {
    area: 'Platform',
    features: [
      {
        name: 'White-label',
        description: 'Hide “Powered by Meteroid” from portals, checkout and emails.',
        availability: 'licensed',
      },
      {
        name: 'License issuing',
        description:
          'Issue signed software licenses to your customers, tied to their subscriptions.',
        availability: 'licensed',
      },
    ],
  },
  {
    area: 'Multi-business',
    summary: 'From one team running several businesses to a platform that bills for others.',
    features: [
      {
        name: 'Several live tenants',
        description:
          'Run several businesses or brands from one team, each Production tenant with its own customers, catalog and invoices.',
        availability: 'licensed',
      },
      {
        name: 'Several organizations',
        description:
          'Host separate teams on one installation, each with its own members and tenants, for example your clients.',
        availability: 'licensed',
      },
      {
        name: 'Connect',
        description:
          'Let the businesses on your platform bill their own customers through you, Stripe Connect style.',
        availability: 'licensed',
      },
    ],
  },
]

const AvailabilityBadge = ({ availability }: { availability: Availability }) =>
  availability === 'free' ? (
    <Badge variant="secondary" size="sm">
      Free
    </Badge>
  ) : (
    <Badge variant="outline" size="sm">
      License
    </Badge>
  )

export const UpgradeTab: FunctionComponent = () => (
  <div className="flex max-w-3xl flex-col gap-6">
    <Card className="grid gap-6 p-6 md:grid-cols-[minmax(0,1fr)_minmax(0,18rem)] md:items-center">
      <div className="flex flex-col gap-4">
        <div className="space-y-1">
          <h3 className="text-base font-semibold tracking-tight">Meteroid Enterprise</h3>
          <p className="text-[13px] leading-snug text-muted-foreground">
            You are running the open-source edition. The Enterprise images add the features below.
          </p>
        </div>
        <ul className="flex flex-col gap-3">
          {HIGHLIGHTS.map(({ icon: Icon, title, description }) => (
            <li key={title} className="flex gap-2.5">
              <Icon size={16} className="mt-0.5 shrink-0 text-brand" />
              <div className="space-y-0.5">
                <div className="text-[13px] font-medium">{title}</div>
                <div className="text-xs leading-snug text-muted-foreground">{description}</div>
              </div>
            </li>
          ))}
        </ul>
        <div className="flex flex-wrap items-center gap-2">
          <Button variant="brand" size="sm" hasIcon asChild>
            <a href={PRICING_URL} target="_blank" rel="noreferrer">
              See editions & pricing <ExternalLinkIcon size={14} />
            </a>
          </Button>
          <Button variant="secondary" size="sm" hasIcon asChild>
            <a href={SWITCH_EDITIONS_DOCS_URL} target="_blank" rel="noreferrer">
              How to switch images <ExternalLinkIcon size={14} />
            </a>
          </Button>
          <Button variant="ghost" size="sm" hasIcon asChild>
            <a href={LICENSE_URL} target="_blank" rel="noreferrer">
              License terms <ExternalLinkIcon size={14} />
            </a>
          </Button>
        </div>
      </div>
      <figure className="hidden overflow-hidden rounded-md border border-border md:block">
        <img
          src="/img/upgrade/intelligence-dashboard.png"
          alt="A Meteroid Intelligence dashboard with ARR, billed revenue, usage and MRR movement charts"
          width={1521}
          height={878}
          loading="lazy"
          className="h-auto w-full"
        />
      </figure>
    </Card>

    {ENTERPRISE_FEATURES.map(({ area, summary, features }) => (
      <SettingsSection key={area} title={area} description={summary}>
        <SettingsRowGroup>
          {features.map(feature => (
            <SettingsRow key={feature.name} title={feature.name} description={feature.description}>
              <AvailabilityBadge availability={feature.availability} />
            </SettingsRow>
          ))}
        </SettingsRowGroup>
      </SettingsSection>
    ))}

    <p className="text-xs text-muted-foreground">
      Free: included in the Meteroid Enterprise images at no cost. License: requires a paid Meteroid
      license.
    </p>
  </div>
)
