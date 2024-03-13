import {
  InputFormField,
  GenericFormField,
  SelectItem,
  SelectFormField,
  Form,
} from '@ui2/components'
import { useAtom } from 'jotai'

import { UncontrolledPriceInput } from '@/components/form/PriceInput'
import {
  componentFeeAtom,
  FeeFormProps,
  EditPriceComponentCard,
} from '@/features/billing/plans/pricecomponents/EditPriceComponentCard'
import { useCurrency } from '@/features/billing/plans/pricecomponents/utils'
import { useZodForm } from '@/hooks/useZodForm'
import { OneTimeFeeSchema, OneTimeFee } from '@/lib/schemas/plans'

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
            <div className="col-span-1 pr-5 border-r border-border">
              <SelectFormField
                control={methods.control}
                className="lg:w-[180px] xl:w-[230px]"
                name="pricing.billingType"
                label="Billing type"
              >
                <SelectItem value="ADVANCE">Paid upfront (advance)</SelectItem>
                <SelectItem value="ARREAR">Postpaid (arrear)</SelectItem>
              </SelectFormField>
            </div>
            <div className="ml-4 col-span-2 space-y-4">
              <InputFormField
                name="pricing.quantity"
                label="Quantity"
                type="number"
                step={1}
                className="max-w-xs"
                control={methods.control}
              />

              <GenericFormField
                name="pricing.unitPrice"
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
