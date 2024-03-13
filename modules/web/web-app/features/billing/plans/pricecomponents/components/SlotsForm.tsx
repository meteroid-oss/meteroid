import {
  FormItem,
  GenericFormField,
  InputFormField,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@ui2/components'
import { useAtom } from 'jotai'
import { useEffect, useState } from 'react'

import { UncontrolledPriceInput } from '@/components/form/PriceInput'
import {
  EditPriceComponentCard,
  FeeFormProps,
  componentFeeAtom,
} from '@/features/billing/plans/pricecomponents/EditPriceComponentCard'
import { TermRateTable } from '@/features/billing/plans/pricecomponents/components/SubscriptionRateForm'
import { useCurrency } from '@/features/billing/plans/pricecomponents/utils'
import { useZodForm } from '@/hooks/useZodForm'
import { Cadence, SlotBased, SlotBasedSchema } from '@/lib/schemas/plans'

export const SlotsForm = (props: FeeFormProps) => {
  const [component] = useAtom(componentFeeAtom)
  const data = component?.data as SlotBased | undefined

  const currency = useCurrency()

  const methods = useZodForm({
    schema: SlotBasedSchema,
    defaultValues: data,
  })

  const [cadence, setCadence] = useState<Cadence | 'COMMITTED'>(
    data && 'cadence' in data.pricing ? data.pricing.cadence : 'COMMITTED'
  )

  useEffect(() => {
    if (cadence === 'COMMITTED') {
      methods.unregister('pricing.cadence')
    } else methods.setValue('pricing.cadence', cadence)
  }, [cadence, methods])

  return (
    <>
      <EditPriceComponentCard submit={methods.handleSubmit(props.onSubmit)} cancel={props.cancel}>
        <div className="grid grid-cols-3 gap-2">
          <div className="col-span-1 pr-5 border-r border-slate-500 space-y-4">
            <FormItem name="cadence" label="Cadence">
              <Select onValueChange={value => setCadence(value as Cadence)} value={cadence}>
                <SelectTrigger className="lg:w-[180px] xl:w-[230px]">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="COMMITTED">Term (variable)</SelectItem>
                  <SelectItem value="MONTHLY">Monthly</SelectItem>
                  <SelectItem value="QUARTERLY">Quarterly</SelectItem>
                  <SelectItem value="ANNUAL">Annual</SelectItem>
                </SelectContent>
              </Select>
            </FormItem>

            <InputFormField
              name="slotUnit.name"
              label="Slot unit"
              control={methods.control}
              className="max-w-xs"
            />
          </div>
          <div className="ml-4 col-span-2 space-y-4">
            {cadence === 'COMMITTED' ? (
              <FormItem name="pricing.price" label="Price">
                <TermRateTable methods={methods} currency={currency} />
              </FormItem>
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
    </>
  )
}
