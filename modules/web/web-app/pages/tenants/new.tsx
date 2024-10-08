import { useMutation } from '@connectrpc/connect-query'
import { Button, Card, Form, InputFormField, SelectFormField, SelectItem } from '@md/ui'
import { ChevronLeft } from 'lucide-react'
import { FunctionComponent } from 'react'
import { useNavigate } from 'react-router-dom'
import { z } from 'zod'

import { useZodForm } from '@/hooks/useZodForm'
import { TenantEnvironmentEnum } from '@/rpc/api/tenants/v1/models_pb'
import { createTenant } from '@/rpc/api/tenants/v1/tenants-TenantsService_connectquery'

const tenantSchema = z.object({
  name: z.string().min(1),
  environment: z.string().transform((val: string, ctx) => {
    try {
      return Number(val) as TenantEnvironmentEnum
    } catch (error) {
      ctx.addIssue({
        code: z.ZodIssueCode.custom,
        message: 'Invalid environment',
        path: ['environment'],
      })
    }
  }),
})

export const TenantNew: FunctionComponent = () => {
  const methods = useZodForm({
    schema: tenantSchema,
    defaultValues: {
      name: '',
      environment: `${TenantEnvironmentEnum.PRODUCTION}` as unknown as TenantEnvironmentEnum,
    },
  })

  const navigate = useNavigate()

  const mut = useMutation(createTenant, {
    onSuccess: ({ tenant }) => {
      navigate(`../${tenant?.slug}`)
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
                await mut.mutateAsync({
                  name: values.name,
                  environment: values.environment,
                })
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
                    name="environment"
                    label="Environment"
                    control={methods.control}
                    placeholder="Select an environment"
                    className="max-w-xs"
                  >
                    <SelectItem value={`${TenantEnvironmentEnum.PRODUCTION}`}>
                      Production
                    </SelectItem>
                    <SelectItem value={`${TenantEnvironmentEnum.STAGING}`}>Staging</SelectItem>
                    <SelectItem value={`${TenantEnvironmentEnum.DEVELOPMENT}`}>
                      Development
                    </SelectItem>
                    <SelectItem value={`${TenantEnvironmentEnum.QA}`}>QA</SelectItem>
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
