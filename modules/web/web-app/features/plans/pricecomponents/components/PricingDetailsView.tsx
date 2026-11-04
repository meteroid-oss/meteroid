import { Badge } from '@md/ui'
import { ColumnDef } from '@tanstack/react-table'
import { useContext } from 'react'

import { SimpleTable } from '@/components/table/SimpleTable'
import {
  formatMetricQuantity,
  formatPerUnit,
  formatRawQuantity,
  type MetricUnit,
  rawQuantity,
  unitPlural,
} from '@/features/billing/metrics/units'
import { PriceComponentProperties } from '@/features/plans/pricecomponents/components/PriceComponentProperty'
import {
  cadenceAdverb,
  cadencePeriod,
  decimalOf,
  type Denomination,
  expirySpan,
  formatAmount,
  formatCreditQuantity,
  hasDuration,
  humanizeKey,
  isIncludedPack,
} from '@/features/plans/pricecomponents/components/pricingFormat'
import { formatPrice } from '@/features/plans/pricecomponents/utils'
import { CreditRateOptionalContext, useCreditUnit } from '@/features/pricing/CreditUnitPriceFields'
import { creditValue, effectiveCreditRate, packCreditPrice } from '@/features/pricing/creditValue'
import { formatCadence } from '@/lib/mapping/prices'
import { CreditPackExpiry_Unit } from '@/rpc/api/prices/v1/models_pb'

import type { CreditUnit } from '@/rpc/api/creditunits/v1/models_pb'
import type {
  CreditPricing,
  Price,
  UsagePricing,
  UsagePricing_MatrixPricing,
  UsagePricing_MatrixPricing_MatrixRow,
  UsagePricing_TieredAndVolumePricing,
} from '@/rpc/api/prices/v1/models_pb'
import type { BillingPeriod } from '@/rpc/api/shared/v1/shared_pb'

interface PricingDetailsViewProps {
  prices: Price[]
  currency: string
  usageUnit?: MetricUnit
  slotUnitName?: string
}

export const PricingDetailsView = ({
  prices,
  currency,
  usageUnit,
  slotUnitName,
}: PricingDetailsViewProps) => {
  const creditUnit = useCreditUnit(prices[0]?.creditUnit?.creditUnitId)
  const deniedAtZero = useContext(CreditRateOptionalContext)
  if (prices.length === 0)
    return <span className="text-muted-foreground text-sm">No pricing data</span>

  const price = prices[0]
  const pricing = price.pricing
  const ownRate = price.creditUnit && Number(price.creditUnit.rate) > 0
  const value = price.creditUnit
    ? effectiveCreditRate(price.creditUnit.rate, creditUnit, currency)
    : undefined
  const denomination: Denomination = price.creditUnit
    ? {
        kind: 'credit',
        currency,
        rate: value,
        precision: creditUnit?.precision,
        names: creditUnit,
      }
    : { kind: 'money', currency }
  const per = `per ${creditUnit?.name ?? 'credit'}, billed ${cadenceAdverb(price.cadence)}`
  const overage =
    denomination.kind === 'credit' ? (
      <PriceComponentProperties
        className="mt-4"
        items={[
          {
            label: 'Overage',
            value:
              !ownRate && deniedAtZero
                ? 'Not billed: access is denied at zero'
                : value
                  ? `${formatPrice(currency)(value)} ${per}${ownRate ? '' : " (the credit's value)"}`
                  : "No money value: set the credit's value or an overage price",
          },
        ]}
      />
    ) : null

  switch (pricing.case) {
    case 'ratePricing':
      return <RatePricingTable prices={prices} currency={currency} />
    case 'slotPricing':
      return <SlotPricingTable prices={prices} currency={currency} unitName={slotUnitName} />
    case 'capacityPricing':
      return (
        <>
          <CapacityPricingTable
            prices={prices}
            currency={currency}
            overage={denomination}
            usageUnit={usageUnit}
          />
          {overage}
        </>
      )
    case 'usagePricing':
      return (
        <>
          <UsagePricingDetail
            pricing={pricing.value}
            denomination={denomination}
            usageUnit={usageUnit}
          />
          {overage}
        </>
      )
    case 'extraRecurringPricing':
      return <SimpleUnitPriceTable unitPrice={pricing.value.unitPrice} currency={currency} />
    case 'oneTimePricing':
      return <SimpleUnitPriceTable unitPrice={pricing.value.unitPrice} currency={currency} />
    case 'creditPricing':
      return (
        <CreditPackDetails pricing={pricing.value} cadence={price.cadence} currency={currency} />
      )
    default:
      return null
  }
}

const CreditPackDetails = ({
  pricing,
  cadence,
  currency,
}: {
  pricing: CreditPricing
  cadence: BillingPeriod
  currency: string
}) => {
  const unit = useCreditUnit(pricing.creditUnitId)
  const money = formatPrice(currency)
  const packs = pricing.quantity || 1
  const charge = decimalOf(pricing.unitPrice).mul(packs)
  const credit = decimalOf(pricing.creditAmount).mul(packs)
  const included = isIncludedPack(pricing)
  const inUnit = Boolean(pricing.creditUnitId)
  const recurrence = pricing.once ? 'once at start' : `/ ${cadencePeriod(cadence)}`

  const creditText = inUnit ? (
    <>
      <strong className="font-semibold text-foreground">
        {formatCreditQuantity(credit, unit?.precision)}
      </strong>{' '}
      {unit ? (credit.eq(1) ? unit.name : unit.pluralName) : 'credits'}
    </>
  ) : (
    <>
      <strong className="font-semibold text-foreground">{money(credit.toFixed())}</strong> of usage
      credit
    </>
  )

  // A money pack shows its bonus; a unit pack its price per credit against the unit's value.
  const bonus =
    !inUnit && !included && credit.gt(charge)
      ? credit.minus(charge).div(charge).mul(100).toDecimalPlaces(1)
      : undefined
  const perCredit =
    inUnit && !included
      ? packCreditPrice(charge.toFixed(), credit.toFixed(), creditValue(unit, currency))
      : undefined

  return (
    <div className="flex flex-col gap-4">
      {included ? (
        <div className="text-sm text-muted-foreground">{creditText}</div>
      ) : (
        <div className="flex flex-wrap items-baseline gap-x-2 gap-y-1">
          <span className="text-2xl font-semibold tabular-nums text-foreground">
            {money(charge.toFixed())}
          </span>
          <span className="text-sm text-muted-foreground">{recurrence}</span>
          <span className="text-sm text-muted-foreground">includes {creditText}</span>
          {bonus && !bonus.isZero() && (
            <Badge
              size="sm"
              className="self-center border-transparent bg-success/10 text-success shadow-none hover:bg-success/10"
            >
              +{bonus.toString()}% bonus
            </Badge>
          )}
          {perCredit && (
            <span className="text-sm text-muted-foreground">
              ≈ {money(perCredit.perCredit)} per {unit?.name ?? 'credit'}
              {perCredit.discountPct !== undefined &&
                perCredit.discountPct > 0 &&
                ` (${perCredit.discountPct}% off ${money(creditValue(unit, currency) ?? '0')})`}
            </span>
          )}
        </div>
      )}
      <PriceComponentProperties
        className="border-t border-border pt-4"
        items={creditPackProperties(pricing, included, unit, currency)}
      />
    </div>
  )
}

const creditPackProperties = (
  pricing: CreditPricing,
  included: boolean,
  unit: CreditUnit | undefined,
  currency: string
) => {
  const { expiry, once } = pricing
  const granted = once
    ? included
      ? 'When the first billing period is invoiced'
      : 'Once, when the first invoice is paid'
    : included
      ? 'When each billing period is invoiced'
      : 'When the invoice is paid'
  const items: { label: string; value: string }[] = [{ label: 'Credit granted', value: granted }]

  if (expiry?.unit === CreditPackExpiry_Unit.BILLING_PERIOD) {
    items.push({
      label: 'Expires',
      value: !included
        ? 'At period end'
        : once
          ? 'End of the first billing period'
          : 'End of each billing period',
    })
  } else if (hasDuration(expiry)) {
    items.push({
      label: 'Expires',
      value: `${expirySpan(expiry)} after ${included ? 'it is granted' : 'payment'}`,
    })
  } else {
    items.push({ label: 'Expires', value: 'Never' })
  }

  items.push({ label: 'Unused credit', value: unusedCredit(pricing, unit, currency) })

  return items
}

const unusedCredit = (
  pricing: CreditPricing,
  unit: CreditUnit | undefined,
  currency: string
): string => {
  const { expiry, rollover } = pricing
  if (!rollover) {
    if (expiry?.unit !== CreditPackExpiry_Unit.BILLING_PERIOD) {
      return hasDuration(expiry) ? 'Stays in the wallet until it expires' : 'Stays in the wallet'
    }
    return pricing.once ? 'Lost when it expires' : 'Resets each period'
  }

  let rolled = 'All of it rolls over'
  if (rollover.maxAmount) {
    const max = decimalOf(rollover.maxAmount)
    if (!pricing.creditUnitId) {
      rolled = `Up to ${formatPrice(currency)(rollover.maxAmount)} rolls over`
    } else {
      const quantity = formatCreditQuantity(max, unit?.precision)
      const name = max.eq(1) ? (unit?.name ?? 'credit') : (unit?.pluralName ?? 'credits')
      rolled = `Up to ${quantity} ${name} ${max.eq(1) ? 'rolls' : 'roll'} over`
    }
  }
  const term = hasDuration(rollover.expiry) ? `, for ${expirySpan(rollover.expiry)}` : ''
  const when =
    expiry?.unit === CreditPackExpiry_Unit.BILLING_PERIOD ? 'to the next period' : 'when it expires'
  return `${rolled} ${when}${term}`
}

const RightAligned = ({ children }: { children: React.ReactNode }) => (
  <div className="text-right tabular-nums">{children}</div>
)

/** The price column of a usage table, plus its money equivalent when priced in credits. */
function amountColumns<T>(
  amountOf: (row: T) => string,
  per: string,
  denomination: Denomination
): ColumnDef<T>[] {
  if (denomination.kind === 'money') {
    const money = formatPrice(denomination.currency)
    return [
      {
        id: 'price',
        header: () => <RightAligned>Price {per}</RightAligned>,
        cell: ({ row }) => <RightAligned>{money(amountOf(row.original))}</RightAligned>,
      },
    ]
  }
  const money = formatPrice(denomination.currency)
  const plural = denomination.names?.pluralName
  const heading = plural ? plural.charAt(0).toUpperCase() + plural.slice(1) : 'Credits'
  const rate = denomination.rate
  const credits: ColumnDef<T> = {
    id: 'credits',
    header: () => (
      <RightAligned>
        {heading} {per}
      </RightAligned>
    ),
    cell: ({ row }) => (
      <RightAligned>
        {formatCreditQuantity(decimalOf(amountOf(row.original)), denomination.precision)}
      </RightAligned>
    ),
  }
  if (!rate) return [credits]
  return [
    credits,
    {
      id: 'equivalent',
      header: () => <RightAligned>{denomination.currency} equivalent</RightAligned>,
      cell: ({ row }) => (
        <RightAligned>
          {money(decimalOf(amountOf(row.original)).mul(decimalOf(rate)).toFixed())}
        </RightAligned>
      ),
    },
  ]
}

const RatePricingTable = ({ prices, currency }: { prices: Price[]; currency: string }) => {
  const money = formatPrice(currency)
  const data = prices.map(p => ({
    term: formatCadence(p.cadence),
    rate: p.pricing.case === 'ratePricing' ? p.pricing.value.rate : undefined,
  }))
  return (
    <SimpleTable
      columns={[
        { header: 'Term', accessorKey: 'term' },
        {
          id: 'price',
          header: () => <RightAligned>Price</RightAligned>,
          cell: ({ row }) => (
            <RightAligned>{row.original.rate ? money(row.original.rate) : '-'}</RightAligned>
          ),
        },
      ]}
      data={data}
    />
  )
}

const SlotPricingTable = ({
  prices,
  currency,
  unitName,
}: {
  prices: Price[]
  currency: string
  unitName?: string
}) => {
  const money = formatPrice(currency)
  const data = prices.map(p => ({
    term: formatCadence(p.cadence),
    unitRate: p.pricing.case === 'slotPricing' ? p.pricing.value.unitRate : undefined,
  }))
  return (
    <SimpleTable
      columns={[
        { header: 'Term', accessorKey: 'term' },
        {
          id: 'price',
          header: () => <RightAligned>Price per {unitName?.toLowerCase() || 'slot'}</RightAligned>,
          cell: ({ row }) => (
            <RightAligned>{row.original.unitRate ? money(row.original.unitRate) : '-'}</RightAligned>
          ),
        },
      ]}
      data={data}
    />
  )
}

const CapacityPricingTable = ({
  prices,
  currency,
  overage,
  usageUnit,
}: {
  prices: Price[]
  currency: string
  overage: Denomination
  usageUnit: MetricUnit | undefined
}) => {
  const money = formatPrice(currency)
  const data = prices.flatMap(p =>
    p.pricing.case === 'capacityPricing'
      ? [
          {
            term: formatCadence(p.cadence),
            rate: p.pricing.value.rate,
            included: p.pricing.value.included,
            overageRate: p.pricing.value.overageRate,
          },
        ]
      : []
  )
  return (
    <SimpleTable
      columns={[
        { header: 'Term', accessorKey: 'term' },
        {
          header: 'Included',
          accessorFn: row =>
            formatMetricQuantity(rawQuantity(row.included, usageUnit) ?? 0, usageUnit),
        },
        {
          id: 'price',
          header: () => <RightAligned>Price</RightAligned>,
          cell: ({ row }) => <RightAligned>{money(row.original.rate)}</RightAligned>,
        },
        {
          id: 'overage',
          header: () => <RightAligned>Overage {formatPerUnit(usageUnit)}</RightAligned>,
          cell: ({ row }) => (
            <RightAligned>{formatAmount(overage, row.original.overageRate)}</RightAligned>
          ),
        },
      ]}
      data={data}
    />
  )
}

const UsagePricingDetail = ({
  pricing,
  denomination,
  usageUnit,
}: {
  pricing: UsagePricing
  denomination: Denomination
  usageUnit: MetricUnit | undefined
}) => {
  const model = pricing.model
  const per = formatPerUnit(usageUnit)
  switch (model.case) {
    case 'perUnit':
      return (
        <SimpleTable
          columns={amountColumns<{ unitPrice: string }>(row => row.unitPrice, per, denomination)}
          data={[{ unitPrice: model.value }]}
        />
      )
    case 'tiered':
    case 'volume':
      return (
        <TieredVolumeTable
          data={model.value}
          denomination={denomination}
          usageUnit={usageUnit}
        />
      )
    case 'package': {
      const blockSize = model.value.blockSize
      const block = formatMetricQuantity(rawQuantity(blockSize, usageUnit) ?? 0, usageUnit)
      return (
        <SimpleTable
          columns={amountColumns<{ packagePrice: string }>(
            row => row.packagePrice,
            `per block of ${block}`,
            denomination
          )}
          data={[model.value]}
        />
      )
    }
    case 'matrix':
      return <MatrixTable data={model.value} denomination={denomination} per={per} />
    default:
      return null
  }
}

interface TierRowDisplay {
  firstUnit: bigint
  lastUnit?: bigint
  unitPrice: string
  flatFee?: string
  flatCap?: string
}

const TieredVolumeTable = ({
  data,
  denomination,
  usageUnit,
}: {
  data: UsagePricing_TieredAndVolumePricing
  denomination: Denomination
  usageUnit: MetricUnit | undefined
}) => {
  const rows: TierRowDisplay[] = data.rows.map((row, idx) => ({
    firstUnit: row.firstUnit,
    lastUnit: idx < data.rows.length - 1 ? data.rows[idx + 1].firstUnit : undefined,
    unitPrice: row.unitPrice,
    flatFee: row.flatFee,
    flatCap: row.flatCap,
  }))

  const hasFlatFee = rows.some(r => r.flatFee != null)
  const hasFlatCap = rows.some(r => r.flatCap != null)
  const units = (value: bigint) => formatRawQuantity(value, usageUnit)

  const columns: ColumnDef<TierRowDisplay>[] = [
    { header: `From (${unitPlural(usageUnit)})`, accessorFn: row => units(row.firstUnit) },
    {
      header: `To (${unitPlural(usageUnit)})`,
      accessorFn: row => (row.lastUnit != null ? units(row.lastUnit) : '∞'),
    },
    ...amountColumns<TierRowDisplay>(row => row.unitPrice, formatPerUnit(usageUnit), denomination),
    ...(hasFlatFee
      ? [
          {
            id: 'flatFee',
            header: () => <RightAligned>Flat fee</RightAligned>,
            cell: ({ row }: { row: { original: TierRowDisplay } }) => (
              <RightAligned>{formatAmount(denomination, row.original.flatFee ?? '0')}</RightAligned>
            ),
          } as ColumnDef<TierRowDisplay>,
        ]
      : []),
    ...(hasFlatCap
      ? [
          {
            id: 'flatCap',
            header: () => <RightAligned>Flat cap</RightAligned>,
            cell: ({ row }: { row: { original: TierRowDisplay } }) => (
              <RightAligned>{formatAmount(denomination, row.original.flatCap ?? '0')}</RightAligned>
            ),
          } as ColumnDef<TierRowDisplay>,
        ]
      : []),
  ]

  return <SimpleTable columns={columns} data={rows} />
}

const MatrixTable = ({
  data,
  denomination,
  per,
}: {
  data: UsagePricing_MatrixPricing
  denomination: Denomination
  per: string
}) => {
  const first = data.rows[0]
  const dimensionColumns: ColumnDef<UsagePricing_MatrixPricing_MatrixRow>[] = [
    {
      id: 'dimension1',
      header: first?.dimension1 ? humanizeKey(first.dimension1.key) : 'Dimension',
      accessorFn: row => row.dimension1?.value ?? '',
    },
    ...(first?.dimension2
      ? [
          {
            id: 'dimension2',
            header: humanizeKey(first.dimension2.key),
            accessorFn: (row: UsagePricing_MatrixPricing_MatrixRow) => row.dimension2?.value ?? '',
          },
        ]
      : []),
  ]

  return (
    <SimpleTable
      columns={[
        ...dimensionColumns,
        ...amountColumns<UsagePricing_MatrixPricing_MatrixRow>(
          row => row.perUnitPrice,
          per,
          denomination
        ),
      ]}
      data={data.rows}
    />
  )
}

const SimpleUnitPriceTable = ({ unitPrice, currency }: { unitPrice: string; currency: string }) => (
  <SimpleTable
    columns={[
      {
        id: 'price',
        header: () => <RightAligned>Unit price</RightAligned>,
        cell: () => <RightAligned>{formatPrice(currency)(unitPrice)}</RightAligned>,
      },
    ]}
    data={[{ unitPrice }]}
  />
)
