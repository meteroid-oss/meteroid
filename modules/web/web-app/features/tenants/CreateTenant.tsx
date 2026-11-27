import { useMutation } from '@connectrpc/connect-query'
import { Form, InputFormField, Modal, SelectFormField, SelectItem } from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { useEffect, useMemo, useState } from 'react'
import { useWatch } from 'react-hook-form'
import { useNavigate, useParams } from 'react-router'
import { toast } from 'sonner'
import { z } from 'zod'

import { entitlementCodes, useEntitlement, useFeaturesQuery } from '@/hooks/useEntitlement'
import { buildSyncParam } from '@/hooks/useSyncQueries'
import { Methods, useZodForm } from '@/hooks/useZodForm'
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

type TenantForm = Methods<typeof tenantSchema>

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

// The selects hold the enums as strings; the schema turns them back into numbers.
const DEFAULT_VALUES = {
  name: '',
  environment: `${TenantEnvironmentEnum.DEVELOPMENT}` as unknown as TenantEnvironmentEnum,
  sandboxPreset: `${SandboxPreset.DEFAULT}` as unknown as SandboxPreset,
}

/**
 * The new-tenant form's state and submit: creates the tenant and opens it. Shared by the
 * modal (from the tenant menu) and the page an organization without tenants lands on.
 */
export const useCreateTenantForm = () => {
  const { organizationSlug } = useParams()
  const navigate = useNavigate()
  const queryClient = useQueryClient()
  const [error, setError] = useState<string | null>(null)

  // Validated as it is typed, so Create enables without leaving the name field first.
  const methods = useZodForm({
    schema: tenantSchema,
    defaultValues: DEFAULT_VALUES,
    mode: 'onChange',
  })

  const mutation = useMutation(createTenant, {
    onError: error => {
      console.error('Error creating tenant:', { error })
      toast.error('Failed to create tenant: ' + error.rawMessage)
      setError(error.rawMessage)
    },
  })

  const submit = methods.handleSubmit(async values => {
    setError(null)
    const result = await mutation.mutateAsync({
      name: values.name,
      environment: values.environment,
      sandboxPreset: values.sandboxPreset,
    })
    if (!result.tenant) return
    // Same as switching tenants from the sidebar: drop the current tenant's cached data.
    queryClient.invalidateQueries()
    // A sandbox's example data is still being generated: have the dashboard refresh its stats.
    const isSandbox = values.environment === TenantEnvironmentEnum.SANDBOX
    const queryParam = isSandbox ? `?${buildSyncParam('stats')}` : ''
    navigate(`/${organizationSlug}/${result.tenant.slug}${queryParam}`)
  })

  const reset = () => {
    methods.reset(DEFAULT_VALUES)
    setError(null)
  }

  return { methods, submit, reset, error, isPending: mutation.isPending }
}

/** Name, environment and, for a sandbox, its example data. */
export const CreateTenantFields = ({ methods }: { methods: TenantForm }) => {
  const environment = useWatch({ control: methods.control, name: 'environment' })
  const sandboxPreset = useWatch({ control: methods.control, name: 'sandboxPreset' })

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
    <div className="w-full space-y-4">
      <InputFormField
        label="Tenant name"
        name="name"
        type="text"
        autoFocus
        placeholder={
          environmentPlaceholders[Number(environment) as TenantEnvironmentEnum] ?? 'Sandbox'
        }
        control={methods.control}
      />

      <div className="space-y-1.5">
        <SelectFormField
          name="environment"
          label="Environment"
          control={methods.control}
          placeholder="Select an environment"
        >
          {canCreateProduction && (
            <SelectItem value={`${TenantEnvironmentEnum.PRODUCTION}`}>Production</SelectItem>
          )}
          <SelectItem value={`${TenantEnvironmentEnum.DEVELOPMENT}`}>Development</SelectItem>
          <SelectItem value={`${TenantEnvironmentEnum.SANDBOX}`}>Sandbox (with data)</SelectItem>
          <SelectItem value={`${TenantEnvironmentEnum.STAGING}`}>Staging</SelectItem>
          <SelectItem value={`${TenantEnvironmentEnum.QA}`}>QA</SelectItem>
        </SelectFormField>
        <p className="text-xs text-muted-foreground">
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
        >
          {presets.map(preset => (
            <SelectItem key={preset} value={`${preset}`}>
              {sandboxPresetLabels[preset]}
            </SelectItem>
          ))}
        </SelectFormField>
      )}
    </div>
  )
}

interface CreateTenantModalProps {
  open: boolean
  onOpenChange: (open: boolean) => void
}

export const CreateTenantModal = ({ open, onOpenChange }: CreateTenantModalProps) => {
  const { methods, submit, reset, error, isPending } = useCreateTenantForm()

  const close = () => {
    onOpenChange(false)
    reset()
  }

  return (
    <Modal
      visible={open}
      header="New tenant"
      size="small"
      onCancel={close}
      onConfirm={() => submit()}
      confirmText="Create tenant"
      confirmDisabled={!methods.formState.isValid}
      loading={isPending}
    >
      <Form {...methods}>
        {/* A real form, so Enter in the name field creates the tenant. */}
        <form
          onSubmit={event => {
            event.preventDefault()
            submit()
          }}
        >
          {/* 12px between the fields and the footer's actions. */}
          <Modal.Content>
            <div className="pb-3">
              <CreateTenantFields methods={methods} />
              {error && <div className="pt-3 text-xs text-destructive">{error}</div>}
            </div>
          </Modal.Content>
          <button type="submit" hidden />
        </form>
      </Form>
    </Modal>
  )
}
