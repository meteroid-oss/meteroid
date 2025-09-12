import { disableQuery } from '@connectrpc/connect-query'
import { Button, ComboboxFormField, Form, GenericFormField, Input } from '@md/ui'
import { ColumnDef } from '@tanstack/react-table'
import { useAtom } from 'jotai'
import { PlusIcon, XIcon } from 'lucide-react'
import { forwardRef, useCallback, useMemo } from 'react'
import { useFieldArray } from 'react-hook-form'
import { useNavigate } from 'react-router-dom'

import { UncontrolledPriceInput } from '@/components/form/PriceInput'
import { SimpleTable } from '@/components/table/SimpleTable'
import { usePlanOverview } from '@/features/plans/hooks/usePlan'
import { EditPriceComponentCard } from '@/features/plans/pricecomponents/EditPriceComponentCard'
import { componentFeeAtom } from '@/features/plans/pricecomponents/atoms'
import { useCurrency } from '@/features/plans/pricecomponents/utils'
import { Methods, useZodForm } from '@/hooks/useZodForm'
import { useQuery } from '@/lib/connectrpc'
import { CapacityFee, CapacityFeeSchema, CapacityThreshold } from '@/lib/schemas/plans'
import { listBillableMetrics } from '@/rpc/api/billablemetrics/v1/billablemetrics-BillableMetricsService_connectquery'

import { FeeFormProps } from './shared'

export const CapacityForm = (props: FeeFormProps) => {
  const [component] = useAtom(componentFeeAtom)
  const currency = useCurrency()

  const navigate = useNavigate()

  const methods = useZodForm({
    schema: CapacityFeeSchema,
    defaultValues: component?.data as CapacityFee,
  })

  // TODO add cadence to capacity. This is the committed cadence, amount & overage is still monthly

  const plan = usePlanOverview()

  const metrics = useQuery(
    listBillableMetrics,
    plan?.productFamilyLocalId
      ? {
          familyLocalId: plan.productFamilyLocalId,
        }
      : disableQuery
  )

  const metricsOptions = useMemo(() => {
    if (!metrics.data?.billableMetrics) return []
    return metrics.data.billableMetrics.map(metric => ({ label: metric.name, value: metric.id }))
  }, [metrics])

  return (
    <>
      <Form {...methods}>
        <EditPriceComponentCard submit={methods.handleSubmit(props.onSubmit)} cancel={props.cancel}>
          <div className="grid grid-cols-3 gap-2">
            <div className="col-span-1 pr-5 border-r border-border space-y-4">
              <ComboboxFormField
                name="metricId"
                label="Billable metric"
                control={methods.control}
                placeholder="Select a metric"
                options={metricsOptions}
                // empty={!metricsOptions.length}
                action={
                  <Button
                    hasIcon
                    variant="ghost"
                    size="full"
                    onClick={() => navigate('add-metric')}
                  >
                    <PlusIcon size={12} /> New metric
                  </Button>
                }
              />
            </div>
            <div className="ml-4 col-span-2">
              <ThresholdTable methods={methods} currency={currency} />
            </div>
          </div>
        </EditPriceComponentCard>
      </Form>
    </>
  )
}

const ThresholdTable = ({
  methods,
  currency,
}: {
  methods: Methods<typeof CapacityFeeSchema> // TODO
  currency: string
}) => {
  const { fields, append, remove } = useFieldArray({
    control: methods.control,
    name: 'thresholds',
  })

  const addThreshold = () => {
    const thresholds = [...fields]

    const lastIncluded = thresholds[thresholds.length - 1]?.includedAmount
    const maxIncluded = lastIncluded ? BigInt(lastIncluded) + BigInt(1) : BigInt(0)

    append({
      includedAmount: maxIncluded,
      perUnitOverage: '',
      price: '',
    })
  }

  const removeThreshold = useCallback(
    (idx: number) => {
      remove(idx)
    },
    [remove]
  )

  const columns = useMemo<ColumnDef<CapacityThreshold>[]>(() => {
    return [
      {
        header: 'Included ',
        cell: ({ row }) => (
          <GenericFormField
            control={methods.control}
            name={`thresholds.${row.index}.includedAmount`}
            render={({ field }) => (
              <IncludedAmountInput {...field} methods={methods} rowIndex={row.index} />
            )}
          />
        ),
      },
      {
        header: 'Tier price',
        cell: ({ row }) => (
          <GenericFormField
            control={methods.control}
            name={`thresholds.${row.index}.price`}
            render={({ field }) => (
              <UncontrolledPriceInput {...field} currency={currency} showCurrency={false} />
            )}
          />
        ),
      },
      {
        header: 'Per unit overage',
        cell: ({ row }) => (
          <GenericFormField
            control={methods.control}
            name={`thresholds.${row.index}.perUnitOverage`}
            render={({ field }) => (
              <UncontrolledPriceInput
                {...field}
                currency={currency}
                showCurrency={false}
                precision={8}
              />
            )}
          />
        ),
      },
      {
        header: '',
        id: 'remove',
        cell: ({ row }) => (
          <Button variant="link" onClick={() => removeThreshold(row.index)}>
            <XIcon size={12} />
          </Button>
        ),
      },
    ]
  }, [methods.control, currency, removeThreshold])

  return (
    <>
      <SimpleTable columns={columns} data={fields} />
      <Button variant="link" onClick={addThreshold}>
        + Add threshold
      </Button>
    </>
  )
}

const IncludedAmountInput = forwardRef<
  HTMLInputElement,
  {
    methods: Methods<typeof CapacityFeeSchema>
    rowIndex: number
    value?: string | number | bigint
    onChange?: (value: string | number | bigint) => void
    onBlur?: () => void
    name?: string
  }
>(({ value, onChange, onBlur, name }, ref) => {
  return (
    <Input
      ref={ref}
      name={name}
      type="number"
      step="1"
      min="0"
      value={value?.toString() || ''}
      onChange={e => onChange?.(e.target.value)}
      onBlur={e => {
        // Ensure it's an integer on blur
        const num = Math.floor(parseFloat(e.target.value) || 0)
        e.target.value = num.toString()
        onChange?.(num.toString())
        onBlur?.()
      }}
    />
  )
})
