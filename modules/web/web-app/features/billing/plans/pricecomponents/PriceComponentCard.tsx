import {
  createConnectQueryKey,
  createProtobufSafeUpdater,
  useMutation,
} from '@connectrpc/connect-query'
import { Button } from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { ColumnDef } from '@tanstack/react-table'
import { useAtom } from 'jotai'
import { ChevronDownIcon, ChevronRightIcon, PencilIcon, Trash2Icon } from 'lucide-react'
import { ReactNode, useCallback, useMemo, useState } from 'react'
import { P, match } from 'ts-pattern'

import { SimpleTable } from '@/components/table/SimpleTable'
import { PriceComponentProperty } from '@/features/billing/plans/pricecomponents/components/PriceComponentProperty'
import {
  editedComponentsAtom,
  formatPrice,
  mapCadence,
  useCurrency,
  useIsDraftVersion,
  usePlanOverview,
} from '@/features/billing/plans/pricecomponents/utils'
import { useQuery } from '@/lib/connectrpc'
import {
  CapacityFee,
  FeeType,
  PriceComponent,
  RateFee,
  SlotFee,
  TieredAndVolumeRow,
  UsageFee,
  UsagePricingModel,
} from '@/lib/schemas/plans'
import { getBillableMetric } from '@/rpc/api/billablemetrics/v1/billablemetrics-BillableMetricsService_connectquery'
import {
  listPriceComponents,
  removePriceComponent,
} from '@/rpc/api/pricecomponents/v1/pricecomponents-PriceComponentsService_connectquery'
import { useConfirmationModal } from 'providers/ConfirmationProvider'

export const PriceComponentCard: React.FC<{
  component: PriceComponent
}> = ({ component }) => {
  const [isCollapsed, setIsCollapsed] = useState(true)

  const overview = usePlanOverview()

  const queryClient = useQueryClient()

  const [, setEditedComponents] = useAtom(editedComponentsAtom)

  const priceElement = useMemo(() => toPriceElements(component.fee), [component])

  const isDraft = useIsDraftVersion()

  const deleteComponentMutation = useMutation(removePriceComponent, {
    onSuccess: () => {
      overview &&
        queryClient.setQueryData(
          createConnectQueryKey(listPriceComponents, { planVersionId: overview.planVersionId }),
          createProtobufSafeUpdater(listPriceComponents, prev => ({
            components: prev?.components.filter(c => c.id !== component.id) ?? [],
          }))
        )
    },
  })

  const showConfirmationModal = useConfirmationModal()

  const removeComponent = async () => {
    showConfirmationModal(() => deleteComponentMutation.mutate({ priceComponentId: component.id }))
  }

  const editComponent = () => {
    setEditedComponents(prev => [...prev, component.id])
  }

  return (
    <div
      className="flex flex-col grow px-4 py-4 border border-border shadow-sm rounded-lg max-w-4xl group bg-card"
      key={component.id}
    >
      <div className="mt-0.5 flex flex-row min-h-9" onClick={() => setIsCollapsed(!isCollapsed)}>
        <div className="flex flex-row items-center cursor-pointer w-full">
          <div className="mr-2">
            {isCollapsed ? (
              <ChevronRightIcon className="w-5 l-5 text-accent-1 group-hover:text-muted-foreground" />
            ) : (
              <ChevronDownIcon className="w-5 l-5 text-accent-1 group-hover:text-muted-foreground" />
            )}
          </div>
          <div className="flex items-center gap-2">
            <h4 className="text-base text-accent-1 font-semibold">{component.name}</h4>
          </div>
        </div>
        {isDraft && (
          <div className="align-end hidden group-hover:flex">
            <Button
              variant="ghost"
              className=" !rounded-r-none bg-transparent text-destructive hover:text-destructive"
              onClick={removeComponent}
              size="icon"
            >
              <Trash2Icon size={12} strokeWidth={2} />
            </Button>
            <Button
              variant="ghost"
              className=" !rounded-l-none"
              onClick={editComponent}
              size="icon"
            >
              <PencilIcon size={12} strokeWidth={2} />
            </Button>
          </div>
        )}
      </div>
      <div className="flex flex-col grow px-7">
        <div className="flex flex-col">
          <div className="grid grid-cols-3 gap-x-6 mt-4">
            <PriceComponentProperty
              label="Pricing model"
              className="col-span-1 border-r border-border pr-4"
            >
              <span>{priceElement?.pricingModel ?? priceElement?.feeType}</span>
            </PriceComponentProperty>
            {priceElement?.linkedItem && (
              <PriceComponentProperty
                label={priceElement.linkedItem.type}
                className="col-span-1 border-r border-border pr-4"
                childrenClassNames="truncate"
              >
                {/* <Link to={`/metrics/${price.metric.id}`} target="_blank" rel="noopener noreferrer">
                  {priceElement.linkedItem.name}
                </Link> */}
                {priceElement.linkedItem.item}
              </PriceComponentProperty>
            )}

            {priceElement?.fixedQuantity && (
              <PriceComponentProperty
                label="Fixed quantity"
                className="col-span-1 pr-4 border-r border-border"
              >
                <span>{priceElement.fixedQuantity}</span>
              </PriceComponentProperty>
            )}
            <PriceComponentProperty
              label="Cadence"
              className="col-span-1 border-r border-border last:border-none pr-4"
            >
              <span>{priceElement?.cadence}</span>
            </PriceComponentProperty>
          </div>
        </div>
        {!isCollapsed && (
          <div className="mt-6 flex flex-col grow">{renderPricingDetails(component.fee)}</div>
        )}
      </div>
    </div>
  )
}

interface PriceElement {
  feeType: string
  linkedItem?: { type: string; item: ReactNode }
  fixedQuantity?: number
  cadence: string
  pricingModel?: string
}

const toPriceElements = (feeType: FeeType): PriceElement | undefined => {
  const mapUsageModel = (model: UsagePricingModel): string =>
    match(model)
      .with({ model: 'tiered' }, () => 'Tiered')
      .with({ model: 'volume' }, () => 'Volume')
      .with({ model: 'package' }, () => 'Package')
      .with({ model: 'per_unit' }, () => 'Per Unit')
      .with({ model: 'matrix' }, () => 'Matrix')
      .exhaustive()

  return match<FeeType, PriceElement | undefined>(feeType ?? undefined)
    .with({ fee: 'rate' }, ({ data }) => ({
      feeType: 'Rate',
      cadence:
        data.rates.length === 1
          ? mapCadence(data.rates[0].term)
          : data.rates.map(a => mapCadence(a.term)).join(' or '),
    }))
    .with({ fee: 'slot' }, ({ data }) => ({
      feeType: 'Slot-based',
      linkedItem: { type: 'Unit type', item: data.slotUnitName },
      cadence:
        data.rates.length === 1
          ? mapCadence(data.rates[0].term)
          : data.rates.map(a => mapCadence(a.term)).join(' or '),
    }))
    .with({ fee: 'capacity' }, ({ data }) => ({
      feeType: 'Committed capacity',
      linkedItem: {
        type: 'Billable Metric',
        item: <DisplayBillableMetric metricId={data.metricId} />,
      },
      cadence: 'Monthly',
    }))
    .with({ fee: 'oneTime' }, ({ data }) => ({
      feeType: 'Fixed fee',
      cadence: `One-time ${data.quantity > 1 ? `(x${data.quantity})` : ''}`,
    }))
    .with({ fee: 'extraRecurring' }, ({ data }) => ({
      feeType: 'Fixed fee',
      cadence: `${data.term ? mapCadence(data.term) : 'Monthly'} ${
        data.quantity > 1 ? `(x${data.quantity})` : ''
      } ${data.billingType === 'ADVANCE' ? '' : '(Postpaid)'}`,
    }))
    .with({ fee: 'usage' }, ({ data }) => ({
      feeType: mapUsageModel(data.model),
      linkedItem: {
        type: 'Billable Metric',
        item: <DisplayBillableMetric metricId={data.metricId} />,
      },
      cadence: 'Monthly',
    }))
    .exhaustive()
}

const DisplayBillableMetric = ({ metricId }: { metricId: string }) => {
  // TODO getById
  const metric = useQuery(getBillableMetric, {
    id: metricId,
  })

  return metric.isLoading ? <></> : <>{metric.data?.billableMetric?.name}</>
}

const DisplayPrice = ({ price }: { price: string }) => {
  const currency = useCurrency()
  const formatWithCurrency = useCallback(formatPrice(currency), [currency])

  return <>{formatWithCurrency(price)}</>
}
const renderPricingDetails = (feeType: FeeType): ReactNode | undefined => {
  console.log('feeType', feeType)
  return match<FeeType, ReactNode>(feeType ?? undefined)
    .with({ fee: 'rate' }, ({ data }) => renderRate(data))
    .with({ fee: 'oneTime' }, ({ data }) => (
      <SimpleTable
        columns={[
          {
            header: 'Unit Price',
            cell: ({ row }) => <DisplayPrice price={row.original.unitPrice} />,
          },
        ]}
        data={[data]}
      />
    ))
    .with({ fee: 'extraRecurring' }, ({ data }) => (
      <SimpleTable
        columns={[
          {
            header: 'Unit Price',
            cell: ({ row }) => <DisplayPrice price={row.original.unitPrice} />,
          },
        ]}
        data={[data]}
      />
    ))
    .with({ fee: 'capacity' }, ({ data }) => renderCommittedCapacity(data))
    .with({ fee: 'slot' }, ({ data }) => renderSlotBased(data))
    .with({ fee: 'usage' }, ({ data }) => renderUsageBased(data))
    .exhaustive()
}

const renderRate = (rate: RateFee) => {
  return (
    <SimpleTable
      columns={[
        {
          header: 'Term',
          accessorKey: 'term',
        },
        { header: 'Price', cell: ({ row }) => <DisplayPrice price={row.original.price} /> },
      ]}
      data={rate.rates}
    />
  )
}

const renderSlotBased = (rate: SlotFee) => {
  return (
    <SimpleTable
      columns={[
        {
          header: 'Term',
          accessorKey: 'term',
        },
        {
          header: 'Price per slot',
          cell: ({ row }) => <DisplayPrice price={row.original.price} />,
        },
      ]}
      data={rate.rates}
    />
  )
}

type TieredAndVolumeRowDisplay = TieredAndVolumeRow & { lastUnit?: bigint }

const renderUsageBased = (rate: UsageFee) => {
  return match(rate.model)
    .with({ model: 'per_unit' }, ({ data }) => (
      <SimpleTable
        columns={[
          {
            header: 'Unit price',
            cell: ({ row }) => <DisplayPrice price={row.original.unitPrice} />,
          },
        ]}
        data={[data]}
      />
    ))
    .with({ model: 'package' }, ({ data }) => (
      <SimpleTable
        columns={[
          { header: 'Block size', accessorKey: 'blockSize' },
          {
            header: 'Block price',
            cell: ({ row }) => <DisplayPrice price={row.original.packagePrice} />,
          },
        ]}
        data={[data]}
      />
    ))
    .with({ model: 'matrix' }, ({ data }) => {
      const dimensionHeaders = data.dimensionRates[0]
        ? [data.dimensionRates[0].dimension1.key, data.dimensionRates[0].dimension2?.key].filter(
            Boolean
          )
        : ['Dimensions']

      return (
        <SimpleTable
          columns={[
            {
              header: dimensionHeaders.join(', '),
              accessorFn: row => {
                const values = [row.dimension1.value]
                if (row.dimension2) values.push(row.dimension2.value)
                return values.join(', ')
              },
            },
            {
              header: 'Unit price',
              cell: ({ row }) => <DisplayPrice price={row.original.price} />,
            },
          ]}
          data={data.dimensionRates}
        />
      )
    })
    .with(P.union({ model: 'tiered' }, { model: 'volume' }), ({ data }) => {
      const hasFlatFee = data.rows.some(row => row.flatFee != null)
      const hasFlatCap = data.rows.some(row => row.flatCap != null)

      const zipWithLastUnit = (rows: TieredAndVolumeRow[]): TieredAndVolumeRowDisplay[] =>
        rows.map((row, idx) => ({
          ...row,
          lastUnit: idx < rows.length - 1 ? rows[idx + 1].firstUnit : undefined,
        }))

      const columns: ColumnDef<TieredAndVolumeRowDisplay>[] = [
        { header: 'First unit', accessorKey: 'firstUnit' },
        { header: 'Last unit', accessorFn: row => row.lastUnit ?? '∞' },
        {
          header: 'Unit price',
          cell: ({ row }) => <DisplayPrice price={row.original.unitPrice} />,
        },
        ...(hasFlatFee
          ? [
              {
                header: 'Flat fee',
                cell: ({ row }) => <DisplayPrice price={row.original.flatFee ?? '0'} />,
              } as ColumnDef<TieredAndVolumeRowDisplay>,
            ]
          : []),
        ...(hasFlatCap
          ? [
              {
                header: 'Flat cap',
                cell: ({ row }) => <DisplayPrice price={row.original.flatCap ?? '0'} />,
              } as ColumnDef<TieredAndVolumeRowDisplay>,
            ]
          : []),
      ]

      return <SimpleTable columns={columns} data={zipWithLastUnit(data.rows)} />
    })
    .exhaustive()
}

const renderCommittedCapacity = (capacity: CapacityFee) => {
  return (
    <SimpleTable
      columns={[
        { header: 'Included Amount', accessorKey: 'includedAmount' },
        { header: 'Price', accessorKey: 'price' },
        { header: 'Per Unit Overage', accessorKey: 'perUnitOverage' },
      ]}
      data={capacity.thresholds}
    />
  )
}
