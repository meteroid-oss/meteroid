import { useMutation } from '@connectrpc/connect-query'
import { Button, Card, Form, InputFormField, SelectFormField, SelectItem } from '@md/ui'
import { ChevronLeft } from 'lucide-react'
import { FunctionComponent, useEffect, useMemo, useState } from 'react'
import { useWatch } from 'react-hook-form'
import { useNavigate, useParams } from 'react-router-dom'
import { toast } from 'sonner'
import { z } from 'zod'

import { entitlementCodes, useEntitlement, useFeaturesQuery } from '@/hooks/useEntitlement'
import { buildSyncParam } from '@/hooks/useSyncQueries'
import { useZodForm } from '@/hooks/useZodForm'
import { useQuery } from '@/lib/connectrpc'
import { TenantEnvironmentEnum } from '@/rpc/api/tenants/v1/models_pb'
import { createTenant, listTenants } from '@/rpc/api/tenants/v1/tenants-TenantsService_connectquery'
import { SandboxPreset } from '@/rpc/api/tenants/v1/tenants_pb'

const tenantSchema = z.object({
  name: z.string().min(3).max(50),
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
  sandboxPreset: z.string().transform((val: string) => Number(val) as SandboxPreset),
})

const environmentPlaceholders: Record<TenantEnvironmentEnum, string> = {
  [TenantEnvironmentEnum.PRODUCTION]: 'Production',
  [TenantEnvironmentEnum.DEVELOPMENT]: 'Development',
  [TenantEnvironmentEnum.SANDBOX]: 'Sandbox',
  [TenantEnvironmentEnum.STAGING]: 'Staging',
  [TenantEnvironmentEnum.QA]: 'QA',
  [TenantEnvironmentEnum.DEMO]: 'Demo',
}

const sandboxPresetLabels: Record<SandboxPreset, string> = {
  [SandboxPreset.DEFAULT]: 'Team collaboration SaaS',
  [SandboxPreset.AI_CREDITS]: 'AI API with prepaid credits (live usage)',
}

export const TenantNew: FunctionComponent = () => {
  const { organizationSlug } = useParams()
  const methods = useZodForm({
    schema: tenantSchema,
    defaultValues: {
      name: '',
      environment: `${TenantEnvironmentEnum.DEVELOPMENT}` as unknown as TenantEnvironmentEnum,
      sandboxPreset: `${SandboxPreset.DEFAULT}` as unknown as SandboxPreset,
    },
  })

  const [error, setError] = useState<string | null>(null)

  const navigate = useNavigate()

  const mut = useMutation(createTenant, {
    onSuccess: ({ tenant }) => {
      navigate(`../${tenant?.slug}`)
    },
    onError: error => {
      console.error('Error creating tenant:', { error })
      toast.error('Failed to create tenant: ' + error.rawMessage)
      setError(error.rawMessage)
    },
  })

  const environment = useWatch({
    control: methods.control,
    name: 'environment',
  })
  const sandboxPreset = useWatch({
    control: methods.control,
    name: 'sandboxPreset',
  })

  const features = useFeaturesQuery().data
  const tenants = useQuery(listTenants).data?.tenants
  const productionCount =
    tenants?.filter(t => t.environment === TenantEnvironmentEnum.PRODUCTION).length ?? 0
  // The first Production tenant comes with the organization (or Cloud's go-live), not from here.
  const canCreateProduction =
    features !== undefined &&
    productionCount > 0 &&
    (features.productionTenantLimit === undefined ||
      productionCount < features.productionTenantLimit)
  const isProduction = Number(environment) === TenantEnvironmentEnum.PRODUCTION

  const adminSandboxes = useEntitlement(entitlementCodes.ADMIN_SANDBOXES)
  const presets = useMemo(
    () =>
      adminSandboxes ? [SandboxPreset.DEFAULT, SandboxPreset.AI_CREDITS] : [SandboxPreset.DEFAULT],
    [adminSandboxes]
  )
  useEffect(() => {
    if (!presets.includes(Number(sandboxPreset))) {
      methods.setValue('sandboxPreset', `${SandboxPreset.DEFAULT}` as unknown as SandboxPreset, {
        shouldValidate: true,
      })
    }
  }, [presets, sandboxPreset, methods])

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
                const result = await mut.mutateAsync({
                  name: values.name,
                  environment: values.environment,
                  sandboxPreset: values.sandboxPreset,
                })
                if (result.tenant) {
                  // Add query param for sandbox tenants to trigger data refresh on dashboard
                  const isSandbox = values.environment === TenantEnvironmentEnum.SANDBOX
                  const queryParam = isSandbox ? `?${buildSyncParam('stats')}` : ''
                  navigate(`/${organizationSlug}/${result.tenant.slug}${queryParam}`)
                }
              })}
            >
              <Card className="px-8 py-6">
                <div className="w-full space-y-4">
                  <div className="flex items-end space-x-2">
                    <InputFormField
                      label="Tenant name"
                      name="name"
                      className="max-w-xs"
                      type="text"
                      placeholder={
                        environmentPlaceholders[environment ?? TenantEnvironmentEnum.SANDBOX]
                      }
                      control={methods.control}
                    />
                  </div>

                  <div className="space-y-1.5">
                    <SelectFormField
                      name="environment"
                      label="Environment"
                      control={methods.control}
                      placeholder="Select an environment"
                      className="max-w-xs"
                    >
                      {canCreateProduction && (
                        <SelectItem value={`${TenantEnvironmentEnum.PRODUCTION}`}>
                          Production
                        </SelectItem>
                      )}
                      <SelectItem value={`${TenantEnvironmentEnum.DEVELOPMENT}`}>
                        Development
                      </SelectItem>
                      <SelectItem value={`${TenantEnvironmentEnum.SANDBOX}`}>
                        Sandbox (with data)
                      </SelectItem>
                      <SelectItem value={`${TenantEnvironmentEnum.STAGING}`}>Staging</SelectItem>
                      <SelectItem value={`${TenantEnvironmentEnum.QA}`}>QA</SelectItem>
                    </SelectFormField>
                    <p className="max-w-xs text-xs text-muted-foreground">
                      {isProduction
                        ? 'A Production tenant bills your customers for real and sends them their emails.'
                        : 'Customer emails start in Test mode: only your team receives them. You can change this in Customer emails.'}
                    </p>
                  </div>

                  {Number(environment) === TenantEnvironmentEnum.SANDBOX && (
                    <SelectFormField
                      name="sandboxPreset"
                      label="Example data"
                      control={methods.control}
                      placeholder="Select an example"
                      className="max-w-xs"
                    >
                      {presets.map(preset => (
                        <SelectItem key={preset} value={`${preset}`}>
                          {sandboxPresetLabels[preset]}
                        </SelectItem>
                      ))}
                    </SelectFormField>
                  )}
                </div>
                {error && <div className="text-destructive text-xs pt-2">{error}</div>}

                <div className="w-full flex justify-end items-center pt-6">
                  <Button
                    variant="ghost"
                    className="!rounded-r-none"
                    size="sm"
                    type="button"
                    onClick={() => navigate(-1)}
                  >
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
