import { ColumnDef } from '@tanstack/react-table'
import { useCallback } from 'react'

import { SimpleTable } from '@/components/table/SimpleTable'
import { formatPrice } from '@/features/plans/pricecomponents/utils'
import { formatCadence } from '@/lib/mapping/prices'

import type {
  Price,
  UsagePricing,
  UsagePricing_MatrixPricing,
  UsagePricing_TieredAndVolumePricing,
} from '@/rpc/api/prices/v1/models_pb'

interface PricingDetailsViewProps {
  prices: Price[]
  currency: string
}

export const PricingDetailsView = ({ prices, currency }: PricingDetailsViewProps) => {
  if (prices.length === 0)
    return <span className="text-muted-foreground text-sm">No pricing data</span>

  const pricing = prices[0].pricing
  switch (pricing.case) {
    case 'ratePricing':
      return <RatePricingTable prices={prices} currency={currency} />
    case 'slotPricing':
      return <SlotPricingTable prices={prices} currency={currency} />
    case 'capacityPricing':
      return <CapacityPricingTable prices={prices} currency={currency} />
    case 'usagePricing':
      return <UsagePricingDetail pricing={pricing.value} currency={currency} />
    case 'extraRecurringPricing':
      return <SimpleUnitPriceTable unitPrice={pricing.value.unitPrice} currency={currency} />
    case 'oneTimePricing':
      return <SimpleUnitPriceTable unitPrice={pricing.value.unitPrice} currency={currency} />
    default:
      return null
  }
}

const DisplayPriceWithCurrency = ({ price, currency }: { price: string; currency: string }) => {
  const fmt = useCallback(formatPrice(currency), [currency])
  return <>{fmt(price)}</>
}

const RatePricingTable = ({ prices, currency }: { prices: Price[]; currency: string }) => {
  const data = prices.map(p => ({
    term: formatCadence(p.cadence),
    rate: p.pricing.case === 'ratePricing' ? p.pricing.value.rate : '-',
  }))
  return (
    <SimpleTable
      columns={[
        { header: 'Term', accessorKey: 'term' },
        {
          header: 'Price',
          cell: ({ row }) => (
            <DisplayPriceWithCurrency price={row.original.rate} currency={currency} />
          ),
        },
      ]}
      data={data}
    />
  )
}

const SlotPricingTable = ({ prices, currency }: { prices: Price[]; currency: string }) => {
  const data = prices.map(p => ({
    term: formatCadence(p.cadence),
    unitRate: p.pricing.case === 'slotPricing' ? p.pricing.value.unitRate : '-',
  }))
  return (
    <SimpleTable
      columns={[
        { header: 'Term', accessorKey: 'term' },
        {
          header: 'Price per slot',
          cell: ({ row }) => (
            <DisplayPriceWithCurrency price={row.original.unitRate} currency={currency} />
          ),
        },
      ]}
      data={data}
    />
  )
}

const CapacityPricingTable = ({ prices, currency }: { prices: Price[]; currency: string }) => {
  const data = prices
    .filter(p => p.pricing.case === 'capacityPricing')
    .map(p => {
      const v = p.pricing.value as { rate: string; included: bigint; overageRate: string }
      return { rate: v.rate, included: v.included, overageRate: v.overageRate }
    })
  return (
    <SimpleTable
      columns={[
        { header: 'Included', accessorFn: row => String(row.included) },
        {
          header: 'Rate',
          cell: ({ row }) => (
            <DisplayPriceWithCurrency price={row.original.rate} currency={currency} />
          ),
        },
        {
          header: 'Overage Rate',
          cell: ({ row }) => (
            <DisplayPriceWithCurrency price={row.original.overageRate} currency={currency} />
          ),
        },
      ]}
      data={data}
    />
  )
}

const UsagePricingDetail = ({
  pricing,
  currency,
}: {
  pricing: UsagePricing
  currency: string
}) => {
  const model = pricing.model
  switch (model.case) {
    case 'perUnit':
      return (
        <SimpleTable
          columns={[
            {
              header: 'Unit price',
              cell: () => <DisplayPriceWithCurrency price={model.value} currency={currency} />,
            },
          ]}
          data={[{ unitPrice: model.value }]}
        />
      )
    case 'tiered':
    case 'volume':
      return <TieredVolumeTable data={model.value} currency={currency} />
    case 'package':
      return (
        <SimpleTable
          columns={[
            { header: 'Block size', accessorFn: row => String(row.blockSize) },
            {
              header: 'Block price',
              cell: ({ row }) => (
                <DisplayPriceWithCurrency
                  price={row.original.packagePrice}
                  currency={currency}
                />
              ),
            },
          ]}
          data={[model.value]}
        />
      )
    case 'matrix':
      return <MatrixTable data={model.value} currency={currency} />
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
  currency,
}: {
  data: UsagePricing_TieredAndVolumePricing
  currency: string
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

  const columns: ColumnDef<TierRowDisplay>[] = [
    { header: 'First unit', accessorFn: row => String(row.firstUnit) },
    {
      header: 'Last unit',
      accessorFn: row => (row.lastUnit != null ? String(row.lastUnit) : '\u221E'),
    },
    {
      header: 'Unit price',
      cell: ({ row }) => (
        <DisplayPriceWithCurrency price={row.original.unitPrice} currency={currency} />
      ),
    },
    ...(hasFlatFee
      ? [
          {
            header: 'Flat fee',
            cell: ({ row }: { row: { original: TierRowDisplay } }) => (
              <DisplayPriceWithCurrency
                price={row.original.flatFee ?? '0'}
                currency={currency}
              />
            ),
          } as ColumnDef<TierRowDisplay>,
        ]
      : []),
    ...(hasFlatCap
      ? [
          {
            header: 'Flat cap',
            cell: ({ row }: { row: { original: TierRowDisplay } }) => (
              <DisplayPriceWithCurrency
                price={row.original.flatCap ?? '0'}
                currency={currency}
              />
            ),
          } as ColumnDef<TierRowDisplay>,
        ]
      : []),
  ]

  return <SimpleTable columns={columns} data={rows} />
}

const MatrixTable = ({
  data,
  currency,
}: {
  data: UsagePricing_MatrixPricing
  currency: string
}) => {
  const dimensionHeaders = data.rows[0]
    ? [data.rows[0].dimension1?.key, data.rows[0].dimension2?.key].filter(Boolean)
    : ['Dimensions']

  return (
    <SimpleTable
      columns={[
        {
          header: dimensionHeaders.join(', '),
          accessorFn: row => {
            const values = [row.dimension1?.value ?? '']
            if (row.dimension2) values.push(row.dimension2.value)
            return values.join(', ')
          },
        },
        {
          header: 'Unit price',
          cell: ({ row }) => (
            <DisplayPriceWithCurrency price={row.original.perUnitPrice} currency={currency} />
          ),
        },
      ]}
      data={data.rows}
    />
  )
}

const SimpleUnitPriceTable = ({
  unitPrice,
  currency,
}: {
  unitPrice: string
  currency: string
}) => (
  <SimpleTable
    columns={[
      {
        header: 'Unit Price',
        cell: () => <DisplayPriceWithCurrency price={unitPrice} currency={currency} />,
      },
    ]}
    data={[{ unitPrice }]}
  />
)
