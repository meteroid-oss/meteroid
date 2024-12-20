import { Form, InputFormField } from '@md/ui'
import { useAtom } from 'jotai'

import { EditPriceComponentCard } from '@/features/plans/pricecomponents/EditPriceComponentCard'
import { TermRateTable } from '@/features/plans/pricecomponents/components/SubscriptionRateForm'
import { useCurrency } from '@/features/plans/pricecomponents/utils'
import { useZodForm } from '@/hooks/useZodForm'
import { SlotFee, SlotFeeSchema } from '@/lib/schemas/plans'

import { componentFeeAtom } from '../atoms'

import { FeeFormProps } from './shared'

export const SlotsForm = (props: FeeFormProps) => {
  const [component] = useAtom(componentFeeAtom)
  const data = component?.data as SlotFee | undefined

  const currency = useCurrency()

  const methods = useZodForm({
    schema: SlotFeeSchema,
    defaultValues: data,
  })

  return (
    <Form {...methods}>
      <EditPriceComponentCard submit={methods.handleSubmit(props.onSubmit)} cancel={props.cancel}>
        <div className="grid grid-cols-3 gap-2">
          <div className="col-span-1 pr-5 border-r border-border space-y-4">
            <InputFormField
              name="slotUnitName"
              label="Slot unit"
              control={methods.control}
              className="max-w-xs"
            />
          </div>
          <div className="ml-4 col-span-2 space-y-4">
            <TermRateTable methods={methods} currency={currency} />
          </div>
        </div>
      </EditPriceComponentCard>
    </Form>
  )
}
