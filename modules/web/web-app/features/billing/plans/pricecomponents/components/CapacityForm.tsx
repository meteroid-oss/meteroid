import { ColumnDef } from '@tanstack/react-table'
import {
  FormItem,
  SelectRoot,
  SelectTrigger,
  SelectValue,
  SelectContent,
  SelectItem,
  ButtonAlt,
  Input,
} from '@ui/components'
import { useAtom } from 'jotai'
import { XIcon } from 'lucide-react'
import { useState, useEffect, useMemo } from 'react'
import { useFieldArray, useWatch } from 'react-hook-form'

import { ControlledSelect } from '@/components/form'
import PriceInput from '@/components/form/PriceInput'
import { SimpleTable } from '@/components/table/SimpleTable'
import {
  componentFeeAtom,
  FeeFormProps,
  EditPriceComponentCard,
} from '@/features/billing/plans/pricecomponents/EditPriceComponentCard'
import { useCurrency } from '@/features/billing/plans/pricecomponents/utils'
import { useZodForm, Methods } from '@/hooks/useZodForm'
import { useQuery } from '@/lib/connectrpc'
import { CapacitySchema, Capacity, Cadence, Threshold } from '@/lib/schemas/plans'
import { listBillableMetrics } from '@/rpc/api/billablemetrics/v1/billablemetrics-BillableMetricsService_connectquery'
import { useTypedParams } from '@/utils/params'

export const CapacityForm = (props: FeeFormProps) => {
  const [component] = useAtom(componentFeeAtom)
  const currency = useCurrency()

  const methods = useZodForm({
    schema: CapacitySchema,
    defaultValues: component?.data as Capacity,
  })

  // TODO add cadence to capacity. This is the committed cadence, amount & overage is still monthly
  // also TODO, it needs to be picked from the db in edit (same for slots / rate)
  const [cadence, setCadence] = useState<Cadence | 'COMMITTED'>('COMMITTED')

  const { familyExternalId } = useTypedParams<{ familyExternalId: string }>()

  const metrics = useQuery(
    listBillableMetrics,
    {
      familyExternalId: familyExternalId!,
    },
    { enabled: !!familyExternalId }
  )

  const metricsOptions = useMemo(() => {
    if (!metrics.data?.billableMetrics) return []
    return metrics.data.billableMetrics.map(metric => ({ label: metric.name, value: metric.id }))
  }, [metrics])

  console.log('errors', methods.formState.errors)
  console.log('values', methods.getValues())

  return (
    <>
      <EditPriceComponentCard submit={methods.handleSubmit(props.onSubmit)} cancel={props.cancel}>
        <div className="grid grid-cols-3 gap-2">
          <div className="col-span-1 pr-5 border-r border-slate-500 space-y-4">
            <FormItem name="cadence" label="Cadence">
              <SelectRoot
                onValueChange={value => setCadence(value as Cadence)}
                defaultValue="COMMITTED"
              >
                <SelectTrigger className="lg:w-[180px] xl:w-[230px]">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent viewportClassName="lg:w-[180px] xl:w-[230px]">
                  <SelectItem value="COMMITTED">Term (variable)</SelectItem>
                  <SelectItem value="MONTHLY">Monthly</SelectItem>
                  <SelectItem value="QUARTERLY">Quarterly</SelectItem>
                  <SelectItem value="ANNUAL">Annual</SelectItem>
                </SelectContent>
              </SelectRoot>
            </FormItem>

            <FormItem name="metric" label="Billable metric" {...methods.withError('metric')}>
              <ControlledSelect
                {...methods.withControl('metric.id')}
                placeholder="Select a metric"
                className="lg:w-[180px] xl:w-[230px]"
              >
                {metricsOptions.map(option => (
                  <SelectItem value={option.value} key={option.value}>
                    {option.label}
                  </SelectItem>
                ))}
              </ControlledSelect>
            </FormItem>
          </div>
          <div className="ml-4 col-span-2">
            {cadence === 'COMMITTED' ? (
              <>Not implemented</> // TODO use column grouping so that we have sub-tds for each term, only for the fixed fee
            ) : (
              <ThresholdTable methods={methods} currency={currency} />
            )}
          </div>
        </div>
      </EditPriceComponentCard>
    </>
  )
}

const ThresholdTable = ({
  methods,
  currency,
}: {
  methods: Methods<typeof CapacitySchema> // TODO
  currency: string
}) => {
  const { fields, append, remove } = useFieldArray({
    control: methods.control,
    name: 'pricing.thresholds',
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

  const columns = useMemo<ColumnDef<Threshold>[]>(() => {
    return [
      {
        header: 'Included ',
        cell: ({ row }) => <IncludedAmountInput methods={methods} rowIndex={row.index} />,
      },
      {
        header: 'Tier price',
        cell: ({ row }) => (
          <PriceInput
            {...methods.withControl(`pricing.thresholds.${row.index}.price`)}
            {...methods.withError(`pricing.thresholds.${row.index}.price`)}
            currency={currency}
            showCurrency={false}
          />
        ),
      },
      {
        header: 'Per unit overage',
        cell: ({ row }) => (
          <PriceInput
            {...methods.withControl(`pricing.thresholds.${row.index}.perUnitOverage`)}
            {...methods.withError(`pricing.thresholds.${row.index}.perUnitOverage`)}
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
          <ButtonAlt type="link" onClick={() => removeThreshold(row.index)}>
            <XIcon size={12} />
          </ButtonAlt>
        ),
      },
    ]
  }, [methods])

  return (
    <>
      <SimpleTable columns={columns} data={fields} />
      <ButtonAlt type="link" onClick={addThreshold}>
        + Add threshold
      </ButtonAlt>
    </>
  )
}

const IncludedAmountInput = ({
  methods,
  rowIndex,
}: {
  methods: Methods<typeof CapacitySchema>
  rowIndex: number
}) => {
  const { setValue, control } = methods
  const prevRowValue = useWatch({
    control,
    name: `pricing.thresholds.${rowIndex - 1}.includedAmount`,
  })
  const thisValue = useWatch({
    control,
    name: `pricing.thresholds.${rowIndex}.includedAmount`,
  })

  useEffect(() => {
    if (rowIndex > 0 && prevRowValue >= thisValue) {
      const updatedValue = prevRowValue + 1
      setValue(`pricing.thresholds.${rowIndex}.includedAmount`, updatedValue)
    }
  }, [prevRowValue, rowIndex, setValue])

  return (
    <Input
      type="number"
      {...methods.register(`pricing.thresholds.${rowIndex}.includedAmount`, {
        valueAsNumber: true,
      })}
      {...methods.withError(`pricing.thresholds.${rowIndex}.includedAmount`)}
    />
  )
}
