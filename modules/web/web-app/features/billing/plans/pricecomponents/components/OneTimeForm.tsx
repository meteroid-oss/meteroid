import { Form, GenericFormField, InputFormField } from '@md/ui'
import { useAtom } from 'jotai'

import { UncontrolledPriceInput } from '@/components/form/PriceInput'
import { EditPriceComponentCard } from '@/features/billing/plans/pricecomponents/EditPriceComponentCard'
import { useCurrency } from '@/features/billing/plans/pricecomponents/utils'
import { useZodForm } from '@/hooks/useZodForm'
import { OneTimeFee, OneTimeFeeSchema } from '@/lib/schemas/plans'

import { componentFeeAtom } from '../atoms'

import { FeeFormProps } from './shared'

export const OneTimeForm = (props: FeeFormProps) => {
  const [component] = useAtom(componentFeeAtom)
  const currency = useCurrency()

  const methods = useZodForm({
    schema: OneTimeFeeSchema,
    defaultValues: component?.data as OneTimeFee,
  })

  console.log('errors', methods.getValues())

  return (
    <>
      <Form {...methods}>
        <EditPriceComponentCard submit={methods.handleSubmit(props.onSubmit)} cancel={props.cancel}>
          <div className="grid grid-cols-3 gap-2">
            <div className="ml-4 col-span-2 space-y-4">
              <InputFormField
                name="quantity"
                label="Quantity"
                type="number"
                step={1}
                className="max-w-xs"
                control={methods.control}
              />

              <GenericFormField
                name="unitPrice"
                label="Price per unit"
                control={methods.control}
                render={({ field }) => (
                  <UncontrolledPriceInput {...field} currency={currency} className="max-w-xs" />
                )}
              />
            </div>
          </div>
        </EditPriceComponentCard>
      </Form>
    </>
  )
}
