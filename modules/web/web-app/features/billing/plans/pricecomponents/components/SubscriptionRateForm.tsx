import {
  Button,
  Form,
  GenericFormField,
  SelectFormField,
  SelectItem,
} from '@md/ui'
import { ColumnDef } from '@tanstack/react-table'
import { useAtom } from 'jotai'
import { useEffect, useMemo, useState } from 'react'
import { useFieldArray, useWatch } from 'react-hook-form'
import { useNavigate } from 'react-router-dom'

import PriceInput, { UncontrolledPriceInput } from '@/components/form/PriceInput'
import { SimpleTable } from '@/components/table/SimpleTable'
import {
  EditPriceComponentCard,
  FeeFormProps,
  componentFeeAtom,
} from '@/features/billing/plans/pricecomponents/EditPriceComponentCard'
import { useBillingPeriods, useCurrency } from '@/features/billing/plans/pricecomponents/utils'
import { Methods, useZodForm } from '@/hooks/useZodForm'
import { BillingPeriod } from '@/lib/mapping'
import {
  SlotBasedSchema,
  SubscriptionRate,
  SubscriptionRateSchema,
  TermRate,
} from '@/lib/schemas/plans'

export const SubscriptionRateForm = (props: FeeFormProps) => {
  const [component] = useAtom(componentFeeAtom)
  const currency = useCurrency()

  const data = component?.data as SubscriptionRate | undefined

  console.log('data', data)

  const methods = useZodForm({
    schema: SubscriptionRateSchema,
    defaultValues: data,
  })

  const cadence = useWatch({ control: methods.control, name: 'pricing.cadence' })

  return (
    <Form {...methods}>
      <EditPriceComponentCard submit={methods.handleSubmit(props.onSubmit)} cancel={props.cancel}>
        <div className="grid grid-cols-3 gap-2">
          <div className="col-span-1 pr-5 border-r border-border">
            <SelectFormField
              name="pricing.cadence"
              label="Cadence"
              control={methods.control}
              className="lg:w-[180px] xl:w-[230px]"
              onValueChange={value =>
                value === 'COMMITTED' && methods.unregister('pricing.cadence')
              }
            >
              <SelectItem value="COMMITTED">Committed</SelectItem>
              <SelectItem value="MONTHLY">Monthly</SelectItem>
              <SelectItem value="QUARTERLY">Quarterly</SelectItem>
              <SelectItem value="ANNUAL">Annual</SelectItem>
            </SelectFormField>
          </div>
          <div className="ml-4 col-span-2">
            {cadence === 'COMMITTED' ? (
              <TermRateTable methods={methods} currency={currency} />
            ) : (
              <>
                <GenericFormField
                  name="pricing.price"
                  label="Price"
                  control={methods.control}
                  render={({ field }) => (
                    <UncontrolledPriceInput {...field} currency={currency} className="max-w-xs" />
                  )}
                />
              </>
            )}
          </div>
        </div>
      </EditPriceComponentCard>
    </Form>
  )
}

export const TermRateTable = ({
  methods,
  currency,
}: {
  methods: Methods<typeof SubscriptionRateSchema> | Methods<typeof SlotBasedSchema> // TODO
  currency: string
}) => {
  const [billingPeriods] = useBillingPeriods()

  const navigate = useNavigate()

  const { fields, append, remove } = useFieldArray({
    control: methods.control as Methods<typeof SubscriptionRateSchema>['control'],
    name: 'pricing.rates',
  })

  const [itemsToAdd, setItemsToAdd] = useState<BillingPeriod[]>([])
  const [itemsToRemove, setItemsToRemove] = useState<BillingPeriod[]>([])

  useEffect(() => {
    if (!billingPeriods) return
    const fieldTerms = new Set(fields.map(field => field.term))
    const billingPeriodSet = new Set(billingPeriods)

    const itemsToAdd = billingPeriods.filter(billingPeriod => !fieldTerms.has(billingPeriod))
    setItemsToAdd(itemsToAdd)

    const itemsToRemove = [...fieldTerms].filter(term => !billingPeriodSet.has(term))
    setItemsToRemove(itemsToRemove)
  }, [billingPeriods, fields, append, remove])

  useEffect(() => {
    if (itemsToAdd.length > 0) {
      const fieldTerms = new Set(fields.map(field => field.term))
      const itemsToAddSet = new Set(itemsToAdd.filter(term => !fieldTerms.has(term)))
      itemsToAddSet.forEach(term => append({ term, price: '' }))
      setItemsToAdd([])
    }
  }, [itemsToAdd])

  useEffect(() => {
    if (itemsToRemove.length > 0) {
      itemsToRemove.forEach(term => {
        const idx = fields.findIndex(field => field.term === term)
        if (idx !== -1) remove(idx)
      })
      setItemsToRemove([])
    }
  }, [itemsToRemove])

  const columns = useMemo<ColumnDef<TermRate>[]>(() => {
    return [
      {
        header: 'Term',
        accessorKey: 'term',
      },
      {
        header: 'Rate',
        cell: ({ row }) => (
          <PriceInput
            {...(methods as Methods<typeof SubscriptionRateSchema>).withControl(
              `pricing.rates.${row.index}.price`
            )}
            {...methods.withError(`pricing.rates.${row.index}.price`)}
            currency={currency}
          />
        ),
      },
    ]
  }, [])

  return (
    <SimpleTable
      columns={columns}
      data={fields}
      emptyMessage={
        <div className="flex items-center justify-between pr-4">
          <div>No Billing terms are not set for this plan.</div>
          <Button variant="ghost" onClick={() => navigate('./billing-terms')}>
            Configure
          </Button>
        </div>
      }
    />
  )
}
