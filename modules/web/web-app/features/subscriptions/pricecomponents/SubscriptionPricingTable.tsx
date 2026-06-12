import { PlainMessage } from '@bufbuild/protobuf'
import { cn } from '@ui/lib'
import { FC } from 'react'

import {
  formatSubscriptionFee,
  formatSubscriptionFeeBillingPeriod,
} from '@/features/subscriptions/utils/fees'
import { SubscriptionFee, SubscriptionFeeBillingPeriod } from '@/rpc/api/subscriptions/v1/models_pb'
import { formatCurrency, formatCurrencyNoRounding } from '@/utils/numbers'

export interface PricingComponent {
  id: string
  name: string
  period: SubscriptionFeeBillingPeriod
  fee?: PlainMessage<SubscriptionFee>
  exampleUsageQuantity?: string
  exampleUsageAmount?: string
}

interface Props {
  components: PricingComponent[]
  currency: string
  className?: string
  labelClassName?: string
  label?: string
}

// Map billing period to display format
// Map subscription fee type to display format
const formatFeeType = (fee: PlainMessage<SubscriptionFee> | undefined) => {
  if (!fee) return 'N/A'

  if (fee.fee.case === 'rate') return 'Rate'
  if (fee.fee.case === 'oneTime') return 'One Time'
  if (fee.fee.case === 'recurring') return 'Recurring'
  if (fee.fee.case === 'capacity') return 'Capacity'
  if (fee.fee.case === 'slot') return 'Slot'
  if (fee.fee.case === 'usage') return 'Usage'

  return 'Unknown'
}

const SubscriptionFeeDetail = ({
  fee,
  currency,
}: {
  fee: PlainMessage<SubscriptionFee> | undefined
  currency: string
}) => {
  if (!fee || !fee.fee.case) {
    return <span className="text-muted-foreground">No fee information</span>
  }

  const formatted = formatSubscriptionFee(fee, currency)

  return (
    <div className="space-y-1">
      <div>
        <span className="font-medium text-foreground text-sm">
          {typeof formatted.amount === 'string'
            ? formatted.amount
            : formatCurrency(Number(formatted.amount), currency || 'USD')}
        </span>
      </div>
      {formatted.breakdown && (
        <div className="text-xs text-muted-foreground whitespace-pre-line mt-1 pl-2 border-l-2 border-muted">
          {formatted.breakdown}
        </div>
      )}
    </div>
  )
}

// Informative example line for usage-based components, e.g.
// "e.g. 10,000 units = €22.83". Amount computed by the backend — not billed.
const UsageExampleLine = ({
  exampleUsageQuantity,
  exampleUsageAmount,
  currency,
}: {
  exampleUsageQuantity: string
  exampleUsageAmount: string | undefined
  currency: string
}) => {
  const quantity = Number(exampleUsageQuantity)
  if (!Number.isFinite(quantity) || !exampleUsageAmount) return null

  return (
    <div className="text-xs text-muted-foreground mt-1 italic">
      e.g. {quantity.toLocaleString()} units ={' '}
      {formatCurrencyNoRounding(exampleUsageAmount, currency || 'USD')}
    </div>
  )
}

// TODO move to quote and rename quote
export const SubscriptionPricingTable: FC<Props> = ({
  components,
  currency,
  className = '',
  labelClassName,
  label = 'Pricing',
}) => {
  if (!components || components.length === 0) {
    return (
      <div className={cn('bg-card rounded-lg shadow-sm', className)}>
        <div className={cn('p-4 border-b border-border', labelClassName)}>
          <h3 className="text-md font-medium text-foreground">{label}</h3>
        </div>
        <div className="p-8 text-center text-muted-foreground">
          No pricing components configured
        </div>
      </div>
    )
  }

  return (
    <div className={cn('bg-card rounded-lg shadow-sm', className)}>
      <div className={cn('p-4 border-b border-border', labelClassName)}>
        <h3 className="text-md font-medium text-foreground">{label}</h3>
      </div>
      <div className="overflow-hidden">
        <table className="w-full">
          <thead className="bg-muted/40">
            <tr>
              <th className="px-4 py-3 text-left text-xs font-medium text-muted-foreground uppercase tracking-wider">
                Name
              </th>
              <th className="px-4 py-3 text-left text-xs font-medium text-muted-foreground uppercase tracking-wider">
                Billing Period
              </th>
              <th className="px-4 py-3 text-left text-xs font-medium text-muted-foreground uppercase tracking-wider">
                Fee Type
              </th>
              <th className="px-4 py-3 text-left text-xs font-medium text-muted-foreground uppercase tracking-wider">
                Price
              </th>
            </tr>
          </thead>
          <tbody>
            {components.map((component, index) => (
              <tr
                key={component.id}
                className={
                  index % 2 === 0 ? 'bg-card' : 'bg-muted/10 border-t border-b border-border'
                }
              >
                <td className="px-4 py-3 text-sm font-medium text-foreground align-top">
                  {component.name}
                </td>
                <td className="px-4 py-3 text-sm text-muted-foreground align-top">
                  {formatSubscriptionFeeBillingPeriod(component.period)}
                </td>
                <td className="px-4 py-3 text-sm text-muted-foreground align-top">
                  {formatFeeType(component.fee)}
                </td>
                <td className="px-4 py-3 text-sm text-muted-foreground align-top">
                  <SubscriptionFeeDetail fee={component.fee} currency={currency} />
                  {component.exampleUsageQuantity && (
                    <UsageExampleLine
                      exampleUsageQuantity={component.exampleUsageQuantity}
                      exampleUsageAmount={component.exampleUsageAmount}
                      currency={currency}
                    />
                  )}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  )
}
