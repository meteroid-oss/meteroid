import { Button, ComboboxFormField, Form, Input } from '@md/ui'
import { ColumnDef } from '@tanstack/react-table'
import { useAtom } from 'jotai'
import { PlusIcon, XIcon } from 'lucide-react'
import { useEffect, useMemo } from 'react'
import { useFieldArray, useWatch } from 'react-hook-form'
import { useNavigate } from 'react-router-dom'

import PriceInput from '@/components/form/PriceInput'
import { SimpleTable } from '@/components/table/SimpleTable'
import { EditPriceComponentCard } from '@/features/plans/pricecomponents/EditPriceComponentCard'
import { componentFeeAtom } from '@/features/plans/pricecomponents/atoms'
import { useCurrency } from '@/features/plans/pricecomponents/utils'
import { Methods, useZodForm } from '@/hooks/useZodForm'
import { useQuery } from '@/lib/connectrpc'
import { CapacityFee, CapacityFeeSchema, CapacityThreshold } from '@/lib/schemas/plans'
import { listBillableMetrics } from '@/rpc/api/billablemetrics/v1/billablemetrics-BillableMetricsService_connectquery'

import { usePlanOverview } from '@/features/plans/hooks/usePlan'
import { disableQuery } from '@connectrpc/connect-query'
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

    const maxIncluded = (thresholds[thresholds.length - 1]?.includedAmount ?? 0) + 1

    append({
      includedAmount: maxIncluded,
      perUnitOverage: '',
      price: '',
    })
  }

  const removeThreshold = (idx: number) => {
    remove(idx)
  }

  const columns = useMemo<ColumnDef<CapacityThreshold>[]>(() => {
    return [
      {
        header: 'Included ',
        cell: ({ row }) => <IncludedAmountInput methods={methods} rowIndex={row.index} />,
      },
      {
        header: 'Tier price',
        cell: ({ row }) => (
          <PriceInput
            {...methods.withControl(`thresholds.${row.index}.price`)}
            {...methods.withError(`thresholds.${row.index}.price`)}
            currency={currency}
            showCurrency={false}
          />
        ),
      },
      {
        header: 'Per unit overage',
        cell: ({ row }) => (
          <PriceInput
            {...methods.withControl(`thresholds.${row.index}.perUnitOverage`)}
            {...methods.withError(`thresholds.${row.index}.perUnitOverage`)}
            currency={currency}
            showCurrency={false}
            precision={8}
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
  }, [methods])

  return (
    <>
      <SimpleTable columns={columns} data={fields} />
      <Button variant="link" onClick={addThreshold}>
        + Add threshold
      </Button>
    </>
  )
}

const IncludedAmountInput = ({
  methods,
  rowIndex,
}: {
  methods: Methods<typeof CapacityFeeSchema>
  rowIndex: number
}) => {
  const { setValue, control } = methods
  const prevRowValue = useWatch({
    control,
    name: `thresholds.${rowIndex - 1}.includedAmount`,
  })
  const thisValue = useWatch({
    control,
    name: `thresholds.${rowIndex}.includedAmount`,
  })

  useEffect(() => {
    if (rowIndex > 0 && prevRowValue >= thisValue) {
      const updatedValue = prevRowValue + 1
      setValue(`thresholds.${rowIndex}.includedAmount`, updatedValue)
    }
  }, [prevRowValue, rowIndex, setValue])

  return (
    <Input
      type="number"
      {...methods.register(`thresholds.${rowIndex}.includedAmount`, {
        valueAsNumber: true,
      })}
      {...methods.withError(`thresholds.${rowIndex}.includedAmount`)}
    />
  )
}
