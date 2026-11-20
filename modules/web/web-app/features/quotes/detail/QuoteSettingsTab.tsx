import { Badge } from '@md/ui'
import { ReactNode } from 'react'
import { Link } from 'react-router-dom'

import { MinimumCommitmentDisplay, productScopeOptions } from '@/features/billing/minimumCommitment'
import { CustomPropertyValue } from '@/features/customproperties/CustomPropertyValue'
import { useCustomPropertyRows } from '@/features/customproperties/useCustomPropertyRows'
import { ResolvedEntitlementsPanel } from '@/features/entitlements/resolved/ResolvedEntitlementsPanel'
import { useCreditUnits } from '@/features/pricing/CreditUnitPriceFields'
import {
  formatSubscriptionFee,
  formatSubscriptionFeeBillingPeriod,
} from '@/features/subscriptions/utils/fees'
import { useBasePath } from '@/hooks/useBasePath'
import { env } from '@/lib/env'
import { CustomPropertyEntityType } from '@/rpc/api/customproperties/v1/models_pb'
import { MinimumCommitment } from '@/rpc/api/plans/v1/models_pb'
import { DetailedQuote, QuoteComponent } from '@/rpc/api/quotes/v1/models_pb'
import { ActivationCondition, PaymentMethodsConfig } from '@/rpc/api/subscriptions/v1/models_pb'
import { parseAndFormatDate } from '@/utils/date'

const Section = ({ title, children }: { title: string; children: ReactNode }) => (
  <section className="flex flex-col gap-2 rounded-lg border border-border bg-card p-4">
    <h3 className="text-[15px] font-medium text-foreground">{title}</h3>
    {children}
  </section>
)

const Detail = ({ label, value, to }: { label: ReactNode; value?: ReactNode; to?: string }) => (
  <div className="flex items-start justify-between gap-4 text-[13px]">
    <dt className="text-muted-foreground">{label}</dt>
    <dd className="text-right text-foreground">
      {to ? (
        <Link to={to} className="text-brand hover:underline">
          {value ?? 'N/A'}
        </Link>
      ) : (
        (value ?? 'N/A')
      )}
    </dd>
  </div>
)

const yesNo = (value: boolean) => (value ? 'Yes' : 'No')

const formatActivationCondition = (condition?: ActivationCondition): string => {
  switch (condition) {
    case ActivationCondition.ON_CHECKOUT:
      return 'On checkout'
    case ActivationCondition.MANUAL:
      return 'Manual'
    default:
      return 'On start date'
  }
}

const formatPaymentMethodsConfig = (config?: PaymentMethodsConfig): string => {
  switch (config?.config.case) {
    case 'bankTransfer':
      return 'Bank transfer'
    case 'external':
      return 'External'
    default:
      return 'Online'
  }
}

const ComponentRow = ({ component, currency }: { component: QuoteComponent; currency: string }) => {
  const creditUnits = useCreditUnits()
  const formatted = formatSubscriptionFee(component.fee, currency, creditUnits)
  return (
    <li className="flex items-start justify-between gap-4 py-2 text-[13px]">
      <div className="min-w-0">
        <div className="font-medium text-foreground">{component.name}</div>
        <div className="text-xs text-muted-foreground">
          {formatSubscriptionFeeBillingPeriod(component.period)}
          {component.isOverride && ' · Custom pricing'}
        </div>
      </div>
      <div className="text-right">
        <div className="text-muted-foreground">{formatted.details}</div>
        <div className="font-medium text-foreground">{formatted.amount}</div>
      </div>
    </li>
  )
}

interface Props {
  quote: DetailedQuote
  minimumCommitment?: MinimumCommitment
  planName?: string
}

/** What the quote sets up beyond the document: the subscription, its invoices, the portal. */
export const QuoteSettingsTab = ({ quote, minimumCommitment, planName }: Props) => {
  const basePath = useBasePath()
  const customPropertyRows = useCustomPropertyRows(
    CustomPropertyEntityType.QUOTE,
    quote.quote?.customProperties
  )
  const q = quote.quote
  if (!q) return null
  const actions = q.customerActions

  return (
    <div className="grid gap-4 xl:grid-cols-2">
      <Section title="Quote">
        <dl className="flex flex-col gap-2">
          <Detail label="Quote number" value={q.quoteNumber} />
          <Detail label="Created" value={parseAndFormatDate(q.createdAt)} />
          <Detail label="Expires" value={q.expiresAt ? parseAndFormatDate(q.expiresAt) : '—'} />
          <Detail label="Currency" value={q.currency} />
          <Detail
            label="Base plan"
            value={planName ?? 'View plan'}
            to={`${basePath}/plan-version/${q.planVersionId}`}
          />
          <Detail
            label="Customer"
            value={quote.customer?.name || q.customerName || 'Customer'}
            to={`${basePath}/customers/${q.customerId}`}
          />
          <Detail label="Customer email" value={quote.customer?.billingEmail} />
          {q.purchaseOrder && <Detail label="Purchase order" value={q.purchaseOrder} />}
        </dl>
      </Section>

      <Section title="Subscription settings">
        <dl className="flex flex-col gap-2">
          <Detail
            label="Start date"
            value={q.startDate ? parseAndFormatDate(q.startDate) : 'Not set (dynamic)'}
          />
          {q.endDate && <Detail label="End date" value={parseAndFormatDate(q.endDate)} />}
          {q.trialDuration !== undefined && q.trialDuration > 0 && (
            <Detail label="Trial" value={`${q.trialDuration} days`} />
          )}
          <Detail
            label="Billing day"
            value={q.billingDayAnchor ? `Day ${q.billingDayAnchor}` : 'Anniversary'}
          />
          <Detail label="Net terms" value={`${q.netTerms} days`} />
          <Detail label="Activation" value={formatActivationCondition(q.activationCondition)} />
          <Detail
            label="Payment method"
            value={formatPaymentMethodsConfig(q.paymentMethodsConfig)}
          />
          <Detail
            label="Auto-create subscription"
            value={yesNo(q.createSubscriptionOnAcceptance)}
          />
        </dl>
      </Section>

      <Section title="Invoice configuration">
        <dl className="flex flex-col gap-2">
          <Detail label="Auto-advance invoices" value={yesNo(q.autoAdvanceInvoices)} />
          <Detail label="Charge automatically" value={yesNo(q.chargeAutomatically)} />
          {q.invoiceMemo && <Detail label="Invoice memo" value={q.invoiceMemo} />}
          {q.invoiceThreshold && <Detail label="Invoice threshold" value={q.invoiceThreshold} />}
        </dl>
      </Section>

      <Section title="Customer portal">
        <dl className="flex flex-col gap-2">
          <Detail label="Questions" value={actions ? yesNo(actions.questions) : 'Yes'} />
          <Detail label="Change requests" value={actions ? yesNo(actions.changeRequests) : 'Yes'} />
          <Detail label="Decline" value={actions ? yesNo(actions.decline) : 'Yes'} />
        </dl>
      </Section>

      {q.convertedToSubscriptionId && (
        <Section title="Linked resources">
          <dl className="flex flex-col gap-2">
            <Detail
              label="Subscription"
              value="View subscription"
              to={`${basePath}/subscriptions/${q.convertedToSubscriptionId}`}
            />
            {q.convertedAt && (
              <Detail label="Converted" value={parseAndFormatDate(q.convertedAt)} />
            )}
          </dl>
        </Section>
      )}

      <Section title="Subscription components">
        {quote.components.length > 0 ? (
          <ul className="divide-y divide-border">
            {quote.components.map(component => (
              <ComponentRow key={component.id} component={component} currency={q.currency} />
            ))}
          </ul>
        ) : (
          <p className="text-[13px] text-muted-foreground">No components</p>
        )}
      </Section>

      {(quote.addOns.length > 0 || quote.coupons.length > 0) && (
        <Section title="Add-ons & discounts">
          {quote.addOns.length > 0 && (
            <div className="flex flex-col gap-1">
              <span className="text-[13px] font-medium text-muted-foreground">Add-ons</span>
              <div className="flex flex-wrap gap-1.5">
                {quote.addOns.map(addOn => (
                  <Badge key={addOn.id} variant="outline" size="sm">
                    {addOn.quantity > 1 ? `${addOn.name} × ${addOn.quantity}` : addOn.name}
                  </Badge>
                ))}
              </div>
            </div>
          )}
          {quote.coupons.length > 0 && (
            <div className="flex flex-col gap-1">
              <span className="text-[13px] font-medium text-muted-foreground">Coupons</span>
              <div className="flex flex-wrap gap-1.5">
                {quote.coupons.map(coupon => (
                  <Badge key={coupon.id} variant="secondary" size="sm">
                    {coupon.couponId}
                  </Badge>
                ))}
              </div>
            </div>
          )}
        </Section>
      )}

      {minimumCommitment && (
        <Section title="Contract terms">
          <MinimumCommitmentDisplay
            commitment={minimumCommitment}
            currency={q.currency}
            components={productScopeOptions(quote.components)}
          />
        </Section>
      )}

      {customPropertyRows.length > 0 && (
        <Section title="Custom properties">
          <dl className="flex flex-col gap-2">
            {customPropertyRows.map(row => (
              <Detail
                key={row.key}
                label={row.label}
                value={<CustomPropertyValue def={row.def} value={row.raw} />}
              />
            ))}
          </dl>
        </Section>
      )}

      {env.entitlementsEnabled && (
        <div className="xl:col-span-2">
          <Section title="Entitlements">
            <ResolvedEntitlementsPanel entity={{ type: 'quote', id: q.id }} canPin={true} />
          </Section>
        </div>
      )}
    </div>
  )
}
