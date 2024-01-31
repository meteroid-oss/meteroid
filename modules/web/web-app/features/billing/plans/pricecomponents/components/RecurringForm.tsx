import { FormItem, SelectItem, Input } from '@ui/components'
import { useAtom } from 'jotai'

import { ControlledSelect } from '@/components/form'
import PriceInput from '@/components/form/PriceInput'
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
      <EditPriceComponentCard submit={methods.handleSubmit(props.onSubmit)} cancel={props.cancel}>
        <div className="grid grid-cols-3 gap-2">
          <div className="col-span-1 pr-5 border-r border-slate-500 space-y-4">
            <FormItem name="cadence" label="cadence">
              <ControlledSelect
                {...methods.withControl('cadence')}
                className="lg:w-[180px] xl:w-[230px]"
              >
                <SelectItem value="MONTHLY">Monthly</SelectItem>
                <SelectItem value="QUARTERLY">Quarterly</SelectItem>
                <SelectItem value="ANNUAL">Annual</SelectItem>
              </ControlledSelect>
            </FormItem>
            <FormItem name="fee.billingType" label="Billing type">
              <ControlledSelect
                {...methods.withControl('fee.billingType')}
                className="lg:w-[180px] xl:w-[230px]"
              >
                <SelectItem value="ADVANCE">Paid upfront (advance)</SelectItem>
                <SelectItem value="ARREAR">Postpaid (arrear)</SelectItem>
              </ControlledSelect>
            </FormItem>
          </div>
          <div className="ml-4 col-span-2 space-y-4">
            <FormItem name="fee.quantity" label="Quantity" {...methods.withError('fee.quantity')}>
              <Input
                {...methods.register('fee.quantity', {
                  valueAsNumber: true,
                })}
                type="number"
                step={1}
                className="max-w-xs"
              />
            </FormItem>
            <FormItem
              name="fee.unitPrice"
              label="Price per unit"
              {...methods.withError('fee.unitPrice')}
            >
              <PriceInput
                {...methods.withControl('fee.unitPrice')}
                currency={currency}
                className="max-w-xs"
              />
            </FormItem>
          </div>
        </div>
      </EditPriceComponentCard>
    </>
  )
}
