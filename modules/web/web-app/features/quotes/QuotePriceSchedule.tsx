import { formatSubscriptionFeeBillingPeriod } from '@/features/subscriptions/utils/fees'
import { parseAndFormatDate } from '@/lib/utils/date'
import { formatCurrency, formatCurrencyNoRounding } from '@/lib/utils/numbers'

import type { QuoteContractValue, QuotePriceStep } from '@/rpc/api/quotes/v1/models_pb'

export const stepStartLabel = (step: Pick<QuotePriceStep, 'startsOn' | 'offsetMonths'>) => {
  if (step.startsOn) return `From ${parseAndFormatDate(step.startsOn)}`
  if (step.offsetMonths === 0) return 'From the start'
  return `After ${step.offsetMonths} ${step.offsetMonths === 1 ? 'month' : 'months'}`
}

/** "Seats: €12.00 per seat · Monthly", "seat: at least 150", "Over this phase: €14,400.00". */
export const stepItems = (step: QuotePriceStep, currency: string, total?: string): string[] => [
  ...(step.plan ? [`Plan: ${step.plan}`] : []),
  ...step.prices.map(line => {
    const price = formatCurrencyNoRounding(line.price, currency)
    const period =
      line.period !== undefined ? formatSubscriptionFeeBillingPeriod(line.period) : undefined
    const amount = [line.unit ? `${price} per ${line.unit}` : price, period]
      .filter(Boolean)
      .join(' · ')
    return line.name ? `${line.name}: ${amount}` : amount
  }),
  ...step.seats.map(seats => `${seats.code}: at least ${seats.slots}`),
  ...(step.minimum
    ? [
        step.minimum.amountSubunit !== undefined
          ? `Minimum commitment: ${formatCurrency(step.minimum.amountSubunit, currency)} per billing period`
          : 'No minimum commitment',
      ]
    : []),
  ...(total !== undefined ? [`Over this phase: ${formatCurrencyNoRounding(total, currency)}`] : []),
]

/** "Total contract value: €30,240.00", "Annual contract value: …", "Year 2: …". */
export const contractItems = (value: QuoteContractValue, currency: string): string[] => {
  const money = (amount: string) => formatCurrencyNoRounding(amount, currency)
  return [
    value.termMonths !== undefined
      ? `Total contract value: ${money(value.total)}`
      : `First ${value.months} months: ${money(value.total)}`,
    ...(value.months > 12 ? [`Annual contract value: ${money(value.annual)}`] : []),
    ...(value.committedTotal !== value.total
      ? [`With minimum commitments: ${money(value.committedTotal)}`]
      : []),
    ...(value.years.length > 1
      ? value.years.map((amount, i) => `Year ${i + 1}: ${money(amount)}`)
      : []),
  ]
}

/** What the quote is worth as billing bills it: fixed fees after discounts, before tax. */
export const QuoteContractValueSection = ({
  value,
  currency,
}: {
  value?: QuoteContractValue
  currency: string
}) => {
  if (!value) return null
  const [headline, ...rest] = contractItems(value, currency)
  return (
    <section className="mb-8">
      <h3 className="border-b border-border pb-3 text-md font-medium text-foreground">
        Contract value
      </h3>
      <div className="space-y-0.5 px-3 pt-3 text-sm sm:px-4">
        <div className="font-medium text-foreground">{headline}</div>
        {rest.map(item => (
          <div key={item} className="text-foreground">
            {item}
          </div>
        ))}
      </div>
      <p className="px-3 pt-2 text-xs text-muted-foreground sm:px-4">
        Fixed fees after discounts and before tax, as billed from the start of billing. Usage is
        billed on top.
      </p>
    </section>
  )
}

/** The quote's ramp as signed: each phase from its day, the prices, seats and minimum it sets. */
export const QuotePriceSchedule = ({
  steps,
  contract,
  currency,
}: {
  steps: QuotePriceStep[]
  contract?: QuoteContractValue
  currency: string
}) => {
  if (steps.length === 0) return null
  return (
    <section className="mb-8">
      <h3 className="border-b border-border pb-3 text-md font-medium text-foreground">
        Price schedule
      </h3>
      <dl className="text-sm">
        {steps.map((step, index) => (
          <div
            key={`${step.offsetMonths}-${index}`}
            className="grid gap-x-6 gap-y-1 border-b border-border px-3 py-3 last:border-b-0 sm:grid-cols-[12rem_1fr] sm:px-4"
          >
            <dt>
              <span className="font-medium text-foreground">{stepStartLabel(step)}</span>
              {step.label && <div className="text-xs text-muted-foreground">{step.label}</div>}
            </dt>
            <dd className="space-y-0.5 text-foreground">
              {stepItems(step, currency, contract?.phases[index]?.total).map(item => (
                <div key={item}>{item}</div>
              ))}
            </dd>
          </div>
        ))}
      </dl>
      <p className="px-3 pt-2 text-xs text-muted-foreground sm:px-4">
        Months count from the start of billing. Usage is billed on top.
      </p>
    </section>
  )
}
