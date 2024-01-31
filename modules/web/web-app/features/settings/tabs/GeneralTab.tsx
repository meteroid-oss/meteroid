import { ButtonAlt, FormItem, Input } from '@ui/components'
import { z } from 'zod'

import { useZodForm } from '@/hooks/useZodForm'

const addressSchema = z.object({
  addressLine1: z.string().min(1),
  addressLine2: z.string().optional(),
  city: z.string().min(1),
  state: z.string().min(1),
  zip: z.string().min(1),
  country: z.string().min(1),
})
const companyTabFormSchema = z.object({
  companyName: z.string().min(1),
  email: z.string(),
  phoneNumber: z.string().optional(),
  address: addressSchema,
})

export const CompanyTab = () => {
  const methods = useZodForm({
    schema: companyTabFormSchema,
  })

  return (
    <div>
      <div className="max-w-3xl space-y-2 px-10 py-6 mt-6 border border-slate-500 rounded-lg bg-white-100 dark:bg-white-200">
        <div className="w-full space-y-4">
          <FormItem label="Legal Company Name" layout="horizontal">
            <Input
              className="max-w-xs"
              type="text"
              placeholder="Company name"
              {...methods.register('companyName')}
            />
          </FormItem>
          <FormItem label="Email" layout="horizontal">
            <Input
              className="max-w-xs"
              type="text"
              placeholder="Email"
              {...methods.register('email')}
            />
          </FormItem>
          <FormItem label="Company Address" layout="horizontal">
            <div className="space-y-2">
              <Input
                type="text"
                placeholder="Address Line 1"
                {...methods.register('address.addressLine1')}
              />
              <Input
                type="text"
                placeholder="Address Line 2"
                {...methods.register('address.addressLine2')}
              />
              <div className="grid grid-cols-2 gap-2">
                <Input type="text" placeholder="City" {...methods.register('address.city')} />
                <Input type="text" placeholder="State" {...methods.register('address.state')} />
                <Input type="text" placeholder="Zip" {...methods.register('address.zip')} />
                <Input type="text" placeholder="Country" {...methods.register('address.country')} />
              </div>
            </div>
          </FormItem>
        </div>

        <div className="w-full flex justify-end pt-6">
          <>Not implemented</>
          <ButtonAlt type="default" className="!rounded-r-none">
            Cancel
          </ButtonAlt>
          <ButtonAlt className="!rounded-l-none" disabled={!methods.formState.isValid}>
            Save
          </ButtonAlt>
        </div>
      </div>
    </div>
  )
}
