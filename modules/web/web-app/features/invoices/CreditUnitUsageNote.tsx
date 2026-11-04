import { cn } from '@md/ui'

import { useCreditUnit } from '@/features/pricing/CreditUnitPriceFields'

import type { LineItem_CreditUnitUsage } from '@/rpc/api/invoices/v1/models_pb'

const formatCredits = (minor: bigint, precision: number) =>
  new Intl.NumberFormat(undefined, { maximumFractionDigits: precision }).format(
    Number(minor) / 10 ** precision
  )

/**
 * The credit side of a line priced in a credit unit: what the usage consumed and what the
 * customer's balance covered. The line's money amount is only the uncovered remainder.
 */
export const CreditUnitUsageNote = ({
  credit,
  estimated = false,
  unitName = 'credits',
  className,
}: {
  credit: LineItem_CreditUnitUsage
  /** A draft's coverage is a projection until finalization draws the credits. */
  estimated?: boolean
  /** The unit's plural name; never its code. */
  unitName?: string
  className?: string
}) => (
  <div className={cn('text-[11px] text-muted-foreground', className)}>
    {estimated && 'Estimated: '}
    {formatCredits(credit.consumed, credit.precision)} {unitName} used,{' '}
    {credit.covered >= credit.consumed
      ? 'all paid with credit'
      : credit.covered <= 0n
        ? 'none paid with credit'
        : `${formatCredits(credit.covered, credit.precision)} paid with credit`}
  </div>
)

/** `CreditUnitUsageNote` naming the unit from the tenant's credit units (dashboard only). */
export const NamedCreditUnitUsageNote = (
  props: Omit<Parameters<typeof CreditUnitUsageNote>[0], 'unitName'>
) => {
  const unit = useCreditUnit(props.credit.creditUnitId)
  return <CreditUnitUsageNote {...props} unitName={unit?.pluralName} />
}
