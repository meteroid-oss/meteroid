import {
  createProtobufSafeUpdater,
  createConnectQueryKey,
  useMutation,
} from '@connectrpc/connect-query'
import { useQueryClient } from '@tanstack/react-query'
import { ColumnDef } from '@tanstack/react-table'
import { Button, TableCell } from '@md/ui'
import { useAtom } from 'jotai'
import { ChevronDownIcon, ChevronRightIcon, PencilIcon, Trash2Icon } from 'lucide-react'
import { ReactNode, useCallback, useMemo, useState } from 'react'
import { match, P } from 'ts-pattern'

import { SimpleTable } from '@/components/table/SimpleTable'
import { PriceComponentProperty } from '@/features/billing/plans/pricecomponents/components/PriceComponentProperty'
import {
  editedComponentsAtom,
  formatPrice,
  useIsDraftVersion,
  mapCadence,
  useCurrency,
  usePlanOverview,
} from '@/features/billing/plans/pricecomponents/utils'
import { useQuery } from '@/lib/connectrpc'
import {
  Capacity,
  FeeType,
  PriceComponent,
  SlotBased,
  SubscriptionRate,
  UsageBased,
  UsagePricingModel,
  TieredAndVolumeRow,
} from '@/lib/schemas/plans'
import { getBillableMetric } from '@/rpc/api/billablemetrics/v1/billablemetrics-BillableMetricsService_connectquery'
import {
  removePriceComponent,
  listPriceComponents,
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

  const computedPrice = useMemo(() => {}, [])

  return (
    <div
      className="flex flex-col grow px-4 py-4 border border-border shadow-sm rounded-lg max-w-4xl group bg-card"
      key={component.id}
    >
      <div className="mt-0.5 flex flex-row min-h-9" onClick={() => setIsCollapsed(!isCollapsed)}>
        <div className="flex flex-row items-center cursor-pointer w-full">
          <div className="mr-2">
            {isCollapsed ? (
              <ChevronRightIcon className="w-5 l-5 text-accent-1 group-hover:text-slate-1000" />
            ) : (
              <ChevronDownIcon className="w-5 l-5 text-accent-1 group-hover:text-slate-1000" />
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
              className="col-span-1 border-r border-slate-600 pr-4"
            >
              <span>{priceElement?.pricingModel ?? priceElement?.feeType}</span>
            </PriceComponentProperty>
            {priceElement?.linkedItem && (
              <PriceComponentProperty
                label={priceElement.linkedItem.type}
                className="col-span-1 border-r border-slate-600 pr-4"
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
                className="col-span-1 pr-4 border-r border-slate-600"
              >
                <span>{priceElement.fixedQuantity}</span>
              </PriceComponentProperty>
            )}
            <PriceComponentProperty
              label="Cadence"
              className="col-span-1 border-r border-slate-600 last:border-none pr-4"
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
  const mapUsageModel = (model: UsagePricingModel['model']): string =>
    match(model)
      .with('tiered', () => 'Tiered')
      .with('volume', () => 'Volume')
      .with('package', () => 'Package')
      .with('per_unit', () => 'Per Unit')
      .exhaustive()

  return match<FeeType, PriceElement | undefined>(feeType ?? undefined)
    .with({ fee: 'rate' }, ({ data }) => ({
      feeType: 'Rate',
      cadence: 'cadence' in data.pricing ? mapCadence(data.pricing.cadence) : 'Committed',
    }))
    .with({ fee: 'slot_based' }, ({ data }) => ({
      feeType: 'Slot-based',
      linkedItem: { type: 'Unit type', item: data.slotUnit?.name },
      cadence: 'cadence' in data.pricing ? mapCadence(data.pricing.cadence) : 'Committed',
    }))
    .with({ fee: 'capacity' }, ({ data }) => ({
      feeType: 'Committed capacity',
      linkedItem: {
        type: 'Billable Metric',
        item: <DisplayBillableMetric metricId={data.metric.id} />,
      },
      cadence: 'thresholds' in data.pricing ? 'Monthly' : 'Committed',
    }))
    .with({ fee: 'one_time' }, ({ data }) => ({
      feeType: 'Fixed fee',
      cadence: `One-time ${data.pricing.billingType === 'ADVANCE' ? '' : '(Postpaid)'}`,
      fixedQuantity: data.pricing.quantity,
    }))
    .with({ fee: 'recurring' }, ({ data }) => ({
      feeType: 'Fixed fee',
      cadence: `${mapCadence(data.cadence)} ${
        data.fee.billingType === 'ADVANCE' ? '' : '(Postpaid)'
      }`,
      fixedQuantity: data.fee.quantity,
    }))
    .with({ fee: 'usage_based' }, ({ data }) => ({
      feeType: mapUsageModel(data.model.model),
      linkedItem: {
        type: 'Billable Metric',
        item: <DisplayBillableMetric metricId={data.metric.id} />,
      },
      cadence: 'Monthly',
    }))
    .otherwise(() => undefined)
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
    .with({ fee: 'one_time' }, ({ data }) => (
      <SimpleTable
        columns={[
          {
            header: 'Unit Price',
            cell: ({ row }) => <DisplayPrice price={row.original.unitPrice} />,
          },
        ]}
        data={[data.pricing]}
      />
    ))
    .with({ fee: 'recurring' }, ({ data }) => (
      <SimpleTable
        columns={[
          {
            header: 'Unit Price',
            cell: ({ row }) => <DisplayPrice price={row.original.unitPrice} />,
          },
        ]}
        data={[data.fee]}
      />
    ))
    .with({ fee: 'capacity' }, ({ data }) => renderCommittedCapacity(data))
    .with({ fee: 'slot_based' }, ({ data }) => renderSlotBased(data))
    .with({ fee: 'usage_based' }, ({ data }) => renderUsageBased(data))
    .exhaustive()
}

const renderRate = (rate: SubscriptionRate) => {
  return match(rate.pricing)
    .with({ rates: P.array() }, ({ rates }) => (
      <SimpleTable
        columns={[
          {
            header: 'Term',
            accessorKey: 'term',
          },
          { header: 'Price', cell: ({ row }) => <DisplayPrice price={row.original.price} /> },
        ]}
        data={rates}
      />
    ))
    .with({ price: P.any }, ({ price }) => (
      <SimpleTable
        columns={[
          { header: 'Price', cell: ({ row }) => <DisplayPrice price={row.original.price} /> },
        ]}
        data={[{ price }]}
      />
    ))
    .exhaustive()
}

const renderSlotBased = (rate: SlotBased) => {
  return match(rate.pricing)
    .with({ rates: P.array() }, ({ rates }) => (
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
        data={rates}
      />
    ))
    .with({ price: P.any }, ({ price }) => (
      <SimpleTable
        columns={[
          {
            header: 'Price per slot',
            cell: ({ row }) => <DisplayPrice price={row.original.price} />,
          },
        ]}
        data={[{ price }]}
      />
    ))
    .exhaustive()
}

const renderUsageBased = (rate: UsageBased) => {
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
            cell: ({ row }) => <DisplayPrice price={row.original.blockPrice} />,
          },
        ]}
        data={[data]}
      />
    ))
    .with(P.union({ model: 'tiered' }, { model: 'volume' }), ({ data }) => {
      const hasFlatFee = data.rows.some(row => row.flatFee != null)
      const hasFlatCap = data.rows.some(row => row.flatCap != null)

      const columns: ColumnDef<TieredAndVolumeRow>[] = [
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
              } as ColumnDef<TieredAndVolumeRow>,
            ]
          : []),
        ...(hasFlatCap
          ? [
              {
                header: 'Flat cap',
                cell: ({ row }) => <DisplayPrice price={row.original.flatCap ?? '0'} />,
              } as ColumnDef<TieredAndVolumeRow>,
            ]
          : []),
      ]

      return <SimpleTable columns={columns} data={data.rows} />
    })
    .exhaustive()
}

const renderCommittedCapacity = (capacity: Capacity) => {
  return match(capacity.pricing)
    .with({ rates: P.array() }, ({ rates }) => {
      const data = rates.flatMap(rate =>
        rate.thresholds.map(threshold => ({ ...threshold, term: rate.term }))
      )

      return (
        <SimpleTable
          columns={[
            {
              header: 'Term',
              accessorKey: 'term',
              // TODO improve
              cell: ({ row, getValue }) => {
                const value = getValue() as ReactNode
                // Determine if this is the first row in the group
                const isFirstRowInGroup =
                  row.index === 0 || data[row.index].term !== data[row.index - 1].term

                if (isFirstRowInGroup) {
                  return <TableCell>{value}</TableCell>
                }
                return null
              },
            },
            { header: 'Included Amount', accessorKey: 'includedAmount' },
            { header: 'Price', accessorKey: 'price' },
            { header: 'Per Unit Overage', accessorKey: 'perUnitOverage' },
          ]}
          data={rates.flatMap(rate =>
            rate.thresholds
              .sort((a, b) => a.includedAmount - b.includedAmount)
              .map(threshold => ({ ...threshold, term: rate.term }))
          )}
        />
      )
    })
    .with({ thresholds: P.array() }, ({ thresholds }) => (
      <SimpleTable
        columns={[
          { header: 'Included Amount', accessorKey: 'includedAmount' },
          { header: 'Price', accessorKey: 'price' },
          { header: 'Per Unit Overage', accessorKey: 'perUnitOverage' },
        ]}
        data={thresholds}
      />
    ))
    .exhaustive()
}
