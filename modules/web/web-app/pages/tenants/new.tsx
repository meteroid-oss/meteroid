import { useMutation } from '@connectrpc/connect-query'
import {
  Button,
  Card,
  CheckboxFormField,
  Form,
  InputFormField,
  SelectFormField,
  SelectItem,
} from '@md/ui'
import { ChevronLeft } from 'lucide-react'
import { FunctionComponent, useEffect, useMemo, useState } from 'react'
import { useWatch } from 'react-hook-form'
import { useNavigate, useParams } from 'react-router'
import { toast } from 'sonner'
import { z } from 'zod'

import { entitlementCodes, useEntitlement, useFeaturesQuery } from '@/hooks/useEntitlement'
import { useIsCloud } from '@/hooks/useIsCloud'
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
  copyFrom: z.string(),
  copyCatalog: z.boolean(),
  copyInvoicing: z.boolean(),
})

const START_EMPTY = 'empty'

const COPIED_LABELS: Record<string, [string, string]> = {
  plan: ['plan', 'plans'],
  product: ['product', 'products'],
  billable_metric: ['metric', 'metrics'],
  add_on: ['add-on', 'add-ons'],
  coupon: ['coupon', 'coupons'],
  feature: ['feature', 'features'],
  credit_unit: ['credit unit', 'credit units'],
  purchase_link: ['purchase link', 'purchase links'],
  quote_template: ['quote template', 'quote templates'],
  custom_property_definition: ['custom property', 'custom properties'],
  invoicing_entity: ['invoicing entity', 'invoicing entities'],
  custom_tax: ['custom tax', 'custom taxes'],
  payment_reminder_sequence: ['reminder sequence', 'reminder sequences'],
}

const copySummary = (copied: { table: string; rows: bigint }[]) => {
  const named = copied.flatMap(({ table, rows }) => {
    const label = COPIED_LABELS[table]
    return label ? [`${rows} ${rows === 1n ? label[0] : label[1]}`] : []
  })
  return named.length > 0 ? named.join(', ') : 'the configuration'
}

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
  [SandboxPreset.CURRENCIES_AND_COUPONS]: 'Plans in three currencies, with coupons',
}

export const TenantNew: FunctionComponent = () => {
  const { organizationSlug } = useParams()
  const methods = useZodForm({
    schema: tenantSchema,
    defaultValues: {
      name: '',
      environment: `${TenantEnvironmentEnum.DEVELOPMENT}` as unknown as TenantEnvironmentEnum,
      sandboxPreset: `${SandboxPreset.DEFAULT}` as unknown as SandboxPreset,
      copyFrom: START_EMPTY,
      copyCatalog: true,
      copyInvoicing: true,
    },
  })

  const [error, setError] = useState<string | null>(null)

  const navigate = useNavigate()

  const mut = useMutation(createTenant, {
    onSuccess: ({ copied }) => {
      if (copied.length > 0) {
        toast.success(`Copied ${copySummary(copied)}`, { duration: 10_000 })
      }
    },
    onError: error => {
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
  const copyFrom = useWatch({
    control: methods.control,
    name: 'copyFrom',
  })
  const isSandbox = Number(environment) === TenantEnvironmentEnum.SANDBOX
  const copying = !isSandbox && copyFrom !== START_EMPTY

  const features = useFeaturesQuery().data
  const tenants = useQuery(listTenants).data?.tenants
  const productionCount =
    tenants?.filter(t => t.environment === TenantEnvironmentEnum.PRODUCTION).length ?? 0
  const isCloud = useIsCloud()
  // On Cloud, the first Production tenant comes from go-live, with its subscription.
  const canCreateProduction =
    features !== undefined &&
    (productionCount > 0 || isCloud === false) &&
    (features.productionTenantLimit === undefined ||
      productionCount < features.productionTenantLimit)
  const isProduction = Number(environment) === TenantEnvironmentEnum.PRODUCTION

  const adminSandboxes = useEntitlement(entitlementCodes.ADMIN_SANDBOXES)
  const presets = useMemo(
    () =>
      adminSandboxes
        ? [SandboxPreset.DEFAULT, SandboxPreset.CURRENCIES_AND_COUPONS, SandboxPreset.AI_CREDITS]
        : [SandboxPreset.DEFAULT, SandboxPreset.CURRENCIES_AND_COUPONS],
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
                const result = await mut
                  .mutateAsync({
                    name: values.name,
                    environment: values.environment,
                    sandboxPreset: values.sandboxPreset,
                    copyFrom: copying
                      ? {
                          tenantId: values.copyFrom,
                          catalog: values.copyCatalog,
                          invoicing: values.copyInvoicing,
                        }
                      : undefined,
                  })
                  .catch(() => undefined)
                if (result?.tenant) {
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

                  {!isSandbox && (tenants?.length ?? 0) > 0 && (
                    <div className="space-y-3">
                      <SelectFormField
                        name="copyFrom"
                        label="Start from"
                        control={methods.control}
                        className="max-w-xs"
                      >
                        <SelectItem value={START_EMPTY}>An empty tenant</SelectItem>
                        {tenants
                          ?.filter(t => t.environment !== TenantEnvironmentEnum.SANDBOX)
                          .map(t => (
                            <SelectItem key={t.id} value={t.id}>
                              A copy of {t.name}
                            </SelectItem>
                          ))}
                      </SelectFormField>
                      {copying && (
                        <div className="space-y-2">
                          <CheckboxFormField
                            control={methods.control}
                            name="copyCatalog"
                            label="Catalog: products, metrics, plans, add-ons, coupons, features, purchase links, custom properties, quote and annex templates"
                          />
                          <CheckboxFormField
                            control={methods.control}
                            name="copyInvoicing"
                            label="Invoicing: entities, bank accounts, taxes, email and reminder settings"
                          />
                          <p className="max-w-sm text-xs text-muted-foreground">
                            Customers, subscriptions, invoices and payments stay where they are.
                            Connectors, webhooks and notification channels hold credentials: set
                            them up again in the new tenant, along with analytics segments, alerts
                            and targets.
                          </p>
                        </div>
                      )}
                    </div>
                  )}

                  {isSandbox && (
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
