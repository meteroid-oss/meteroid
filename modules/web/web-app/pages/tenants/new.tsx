import { useMutation } from '@connectrpc/connect-query'
import { Button, Card, Form, InputFormField, SelectFormField, SelectItem } from '@md/ui'
import { ChevronLeft } from 'lucide-react'
import { FunctionComponent } from 'react'
import { useNavigate } from 'react-router-dom'
import { z } from 'zod'

import { useZodForm } from '@/hooks/useZodForm'
import { createTenant } from '@/rpc/api/tenants/v1/tenants-TenantsService_connectquery'

const tenantSchema = z.object({
  name: z.string().min(1),
  currency: z.string().min(1),
  preset: z.string(),
})

export const TenantNew: FunctionComponent = () => {
  const methods = useZodForm({
    schema: tenantSchema,
  })

  const navigate = useNavigate()

  const mut = useMutation(createTenant, {
    onSuccess: ({ tenant }) => {
      navigate(`/tenant/${tenant?.slug}`)
    },
  })

  return (
    <main className="flex  flex-col flex-1 w-full max-w-screen-2xl pl-8 pr-2 mx-auto h-full overflow-x-hidden ">
      <div className="pt-4">
        <div className="flex space-x-4 items-center">
          <Button variant="ghost" onClick={() => navigate(-1)} className="">
            <ChevronLeft size={16} />
          </Button>

          <div className="text-lg font-semibold">New tenant</div>
        </div>
      </div>

      <div className="relative py-8 px-4 max-w-screen-sm">
        <div className="  space-y-6 w-full h-full overflow-x-hidden">
          <Form {...methods}>
            <form
              onSubmit={methods.handleSubmit(async values => {
                const slug = values.name.toLowerCase().replace(/\s/g, '-')
                await mut.mutateAsync({ ...values, slug })
              })}
            >
              <Card className="px-8 py-6">
                <div className="w-full space-y-4">
                  <InputFormField
                    label="Tenant name"
                    name="name"
                    className="max-w-xs"
                    type="text"
                    placeholder="Production"
                    control={methods.control}
                  />

                  <SelectFormField
                    name="currency"
                    label="Currency"
                    control={methods.control}
                    placeholder="Select a currency"
                    className="max-w-xs"
                  >
                    <SelectItem value="EUR">EUR</SelectItem>
                    <SelectItem value="USD">USD</SelectItem>
                  </SelectFormField>

                  <SelectFormField
                    name="preset"
                    label="Preset"
                    control={methods.control}
                    className="max-w-xs"
                  >
                    <SelectItem value="none">No preset</SelectItem>
                    <SelectItem value="saas">SaaS</SelectItem>
                    <SelectItem value="openstack">OpenStack</SelectItem>
                  </SelectFormField>
                </div>

                <div className="w-full flex justify-end items-center pt-6">
                  <Button variant="ghost" className="!rounded-r-none" size="sm">
                    Cancel
                  </Button>
                  <Button
                    className="!rounded-l-none"
                    size="sm"
                    disabled={!methods.formState.isValid}
                  >
                    Save
                  </Button>
                </div>
              </Card>
            </form>
          </Form>
        </div>
      </div>
    </main>
  )
}
