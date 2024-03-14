import {
  Form,
  GenericFormField,
  InputFormField,
  SelectFormField,
  SelectItem,
} from '@md/ui'
import { useAtom } from 'jotai'
import { useWatch } from 'react-hook-form'

import { UncontrolledPriceInput } from '@/components/form/PriceInput'
import {
  EditPriceComponentCard,
  FeeFormProps,
  componentFeeAtom,
} from '@/features/billing/plans/pricecomponents/EditPriceComponentCard'
import { TermRateTable } from '@/features/billing/plans/pricecomponents/components/SubscriptionRateForm'
import { useCurrency } from '@/features/billing/plans/pricecomponents/utils'
import { useZodForm } from '@/hooks/useZodForm'
import { SlotBased, SlotBasedSchema } from '@/lib/schemas/plans'

export const SlotsForm = (props: FeeFormProps) => {
  const [component] = useAtom(componentFeeAtom)
  const data = component?.data as SlotBased | undefined

  const currency = useCurrency()

  const methods = useZodForm({
    schema: SlotBasedSchema,
    defaultValues: data,
  })

  const cadence = useWatch({ control: methods.control, name: 'pricing.cadence' })

  return (
    <Form {...methods}>
      <EditPriceComponentCard submit={methods.handleSubmit(props.onSubmit)} cancel={props.cancel}>
        <div className="grid grid-cols-3 gap-2">
          <div className="col-span-1 pr-5 border-r border-border space-y-4">
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

            <InputFormField
              name="slotUnit.name"
              label="Slot unit"
              control={methods.control}
              className="max-w-xs"
            />
          </div>
          <div className="ml-4 col-span-2 space-y-4">
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
