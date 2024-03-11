import {
  FormItem,
  SelectItem,
  FormSelect,
  FormInput,
  GenericFormField,
  Form,
} from '@ui2/components'
import { useAtom } from 'jotai'

import { ControlledSelect } from '@/components/form'
import { UncontrolledPriceInput } from '@/components/form/PriceInput'
import {
  componentFeeAtom,
  FeeFormProps,
  EditPriceComponentCard,
} from '@/features/billing/plans/pricecomponents/EditPriceComponentCard'
import { useCurrency } from '@/features/billing/plans/pricecomponents/utils'
import { useZodForm } from '@/hooks/useZodForm'
import { RecurringFixedFeeSchema, RecurringFixedFee } from '@/lib/schemas/plans'

export const RecurringForm = (props: FeeFormProps) => {
  const [component] = useAtom(componentFeeAtom)
  const currency = useCurrency()

  const methods = useZodForm({
    schema: RecurringFixedFeeSchema,
    defaultValues: component?.data as RecurringFixedFee,
  })

  return (
    <>
      <Form {...methods}>
        <EditPriceComponentCard submit={methods.handleSubmit(props.onSubmit)} cancel={props.cancel}>
          <div className="grid grid-cols-3 gap-2">
            <div className="col-span-1 pr-5 border-r border-slate-500 space-y-4">
              <FormSelect
                name="cadence"
                label="Cadence"
                control={methods.control}
                className="lg:w-[180px] xl:w-[230px]"
              >
                <SelectItem value="MONTHLY">Monthly</SelectItem>
                <SelectItem value="QUARTERLY">Quarterly</SelectItem>
                <SelectItem value="ANNUAL">Annual</SelectItem>
              </FormSelect>
              <FormSelect
                name="fee.billingType"
                label="Billing type"
                control={methods.control}
                className="lg:w-[180px] xl:w-[230px]"
              >
                <SelectItem value="ADVANCE">Paid upfront (advance)</SelectItem>
                <SelectItem value="ARREAR">Postpaid (arrear)</SelectItem>
              </FormSelect>
            </div>
            <div className="ml-4 col-span-2 space-y-4">
              <FormInput
                name="fee.quantity"
                label="Quantity"
                type="number"
                step={1}
                className="max-w-xs"
                control={methods.control}
              />
              <GenericFormField
                name="fee.unitPrice"
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
