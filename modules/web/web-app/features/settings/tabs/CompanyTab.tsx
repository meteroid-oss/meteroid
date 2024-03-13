import { z } from 'zod'

import { useZodForm } from '@/hooks/useZodForm'
import { Button, Card, Form, InputFormField, Label } from '@ui2/components'

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
      <Form {...methods}>
        <form
          onSubmit={methods.handleSubmit(async values => {
            alert('Not implemented')
          })}
        >
          <Card className="px-8 py-6">
            <div className="w-full space-y-4">
              <InputFormField
                label="Legal Company Name"
                name="companyName"
                layout="horizontal"
                className="max-w-xs"
                type="text"
                placeholder="Company name"
                control={methods.control}
              />

              <InputFormField
                label="Email"
                layout="horizontal"
                className="max-w-xs"
                type="text"
                placeholder="Email"
                control={methods.control}
                name="email"
              />

              <div className="space-y-0 grid gap-2 md:grid md:grid-cols-12">
                <Label className="col-span-4 text-muted-foreground">Company Address</Label>
                <div className="col-span-4 space-y-3">
                  <div className="space-y-3">
                    <InputFormField
                      placeholder="Address Line 1"
                      control={methods.control}
                      name="address.addressLine1"
                    />
                    <InputFormField
                      placeholder="Address Line 2"
                      control={methods.control}
                      name="address.addressLine2"
                    />
                  </div>
                  <div className="grid grid-cols-2 gap-3">
                    <InputFormField
                      placeholder="City"
                      control={methods.control}
                      name="address.city"
                    />
                    <InputFormField
                      placeholder="State"
                      control={methods.control}
                      name="address.state"
                    />
                    <InputFormField
                      placeholder="Zip"
                      control={methods.control}
                      name="address.zip"
                    />
                    <InputFormField
                      placeholder="Country"
                      control={methods.control}
                      name="address.country"
                    />
                  </div>
                </div>
              </div>
            </div>

            <div className="w-full flex justify-end items-center pt-6">
              <small className="text-muted-foreground">Not implemented</small>
              <Button variant="ghost" className="!rounded-r-none" size="sm">
                Cancel
              </Button>
              <Button className="!rounded-l-none" size="sm" disabled={!methods.formState.isValid}>
                Save
              </Button>
            </div>
          </Card>
        </form>
      </Form>
    </div>
  )
}
