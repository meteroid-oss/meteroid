import { Form, GenericFormField, InputFormField, SelectFormField, SelectItem } from '@md/ui'
import { useAtom } from 'jotai'

import { UncontrolledPriceInput } from '@/components/form/PriceInput'
import { EditPriceComponentCard } from '@/features/plans/pricecomponents/EditPriceComponentCard'
import { useCurrency } from '@/features/plans/pricecomponents/utils'
import { useZodForm } from '@/hooks/useZodForm'
import { ExtraRecurringFee, ExtraRecurringFeeSchema } from '@/lib/schemas/plans'

import { componentFeeAtom } from '../atoms'

import { FeeFormProps } from './shared'

export const RecurringForm = (props: FeeFormProps) => {
  const [component] = useAtom(componentFeeAtom)
  const currency = useCurrency()

  const methods = useZodForm({
    schema: ExtraRecurringFeeSchema,
    defaultValues: component?.data as ExtraRecurringFee,
  })

  return (
    <>
      <Form {...methods}>
        <EditPriceComponentCard submit={methods.handleSubmit(props.onSubmit)} cancel={props.cancel}>
          <div className="grid grid-cols-3 gap-2">
            <div className="col-span-1 pr-5 border-r border-border space-y-4">
              <SelectFormField
                name="term"
                label="Cadence"
                control={methods.control}
                className="lg:w-[180px] xl:w-[230px]"
              >
                <SelectItem value="MONTHLY">Monthly</SelectItem>
                <SelectItem value="QUARTERLY">Quarterly</SelectItem>
                <SelectItem value="ANNUAL">Annual</SelectItem>
              </SelectFormField>
              <SelectFormField
                name="billingType"
                label="Billing type"
                control={methods.control}
                className="lg:w-[180px] xl:w-[230px]"
              >
                <SelectItem value="ADVANCE">Paid upfront (advance)</SelectItem>
                <SelectItem value="ARREAR">Postpaid (arrear)</SelectItem>
              </SelectFormField>
            </div>
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
