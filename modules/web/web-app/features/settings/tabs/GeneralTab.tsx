import { useMutation } from '@connectrpc/connect-query'
import {
  Button,
  ComboboxFormField,
  Form,
  InputFormField,
  MultiSelectFormField,
  MultiSelectItem,
  Switch,
} from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { useEffect } from 'react'
import { useWatch } from 'react-hook-form'
import { useNavigate } from 'react-router-dom'
import { toast } from 'sonner'
import { z } from 'zod'

import { InlineLoading } from '@/components/Loading'
import { Slot } from '@/edition'
import { DeliveryModeBadge } from '@/features/customer-emails/DeliveryView'
import { DangerZone } from '@/features/settings/DangerZone'
import {
  SettingsFieldGrid,
  SettingsRow,
  SettingsRowGroup,
  SettingsSection,
} from '@/features/settings/components/SettingsSection'
import { useBasePath } from '@/hooks/useBasePath'
import { useOrganizationSlug } from '@/hooks/useOrganization'
import { useZodForm } from '@/hooks/useZodForm'
import { useQuery } from '@/lib/connectrpc'
import { generalSettingsSchema, type GeneralSettingsSchema } from '@/lib/schemas/settings'
import { getCurrencies } from '@/rpc/api/instance/v1/instance-InstanceService_connectquery'
import {
  activeTenant,
  listTenantCurrencies,
  updateTenant,
  updateTenantAvailableCurrencies,
} from '@/rpc/api/tenants/v1/tenants-TenantsService_connectquery'

const currencySchema = z.object({
  selectedCurrencies: z.array(z.string()).min(1, 'At least one currency is required'),
})

export const GeneralTab = () => {
  const activeTenantQuery = useQuery(activeTenant)
  const activeTenantData = activeTenantQuery.data

  const getCurrenciesQuery = useQuery(getCurrencies)
  const currencies = getCurrenciesQuery.data?.currencies ?? []

  const activeCurrenciesQuery = useQuery(listTenantCurrencies)
  const activeCurrencies = activeCurrenciesQuery.data?.currencies ?? []

  const queryClient = useQueryClient()
  const navigate = useNavigate()

  const organizationSlug = useOrganizationSlug()
  const basePath = useBasePath()

  const updateTenantMut = useMutation(updateTenant, {
    onSuccess: async res => {
      if (res.tenant) {
        const newSlug = res.tenant.slug
        const hasChanged = newSlug !== activeTenantData?.tenant?.slug

        await queryClient.invalidateQueries()
        toast.success('Updated successfully !')

        if (hasChanged) {
          navigate(`/${organizationSlug}/${newSlug}/settings/general`, { replace: true })
        }
      }
    },
    onError: error => {
      toast.error(error.rawMessage)
    },
  })

  const currencyMethods = useZodForm({
    schema: currencySchema,
    defaultValues: {
      selectedCurrencies: [],
    },
    mode: 'onSubmit',
  })

  const updateTenantCurrencyMut = useMutation(updateTenantAvailableCurrencies, {
    onSuccess: async () => {
      await queryClient.invalidateQueries()
      toast.success('Currencies updated successfully!')
      currencyMethods.reset({
        selectedCurrencies: [],
      })
    },
    onError: error => {
      toast.error(error.rawMessage)
    },
  })

  const methods = useZodForm({
    schema: generalSettingsSchema,
    defaultValues: {
      tradeName: '',
      tenantName: '',
      slug: '',
      reportingCurrency: '',
      alertEvaluationInterval: '0',
      keepLastEvents: false,
    },
    mode: 'onSubmit',
  })

  const keepLastEvents = useWatch({
    control: methods.control,
    name: 'keepLastEvents',
  })

  useEffect(() => {
    if (activeTenantData?.tenant && currencies.length > 0) {
      methods.reset({
        tradeName: activeTenantData.tradeName,
        tenantName: activeTenantData.tenant.name,
        slug: activeTenantData.tenant.slug,
        reportingCurrency: activeTenantData.tenant.reportingCurrency,
        alertEvaluationInterval: String(activeTenantData.tenant.alertEvaluationIntervalSecs ?? 0),
        keepLastEvents: activeTenantData.tenant.keepLastEvents,
      })
    }
  }, [activeTenantData, currencies])

  useEffect(() => {
    if (activeCurrencies.length && currencies.length > 0) {
      currencyMethods.reset({
        selectedCurrencies: activeCurrencies,
      })
    }
  }, [activeCurrencies, currencies])

  if (
    activeTenantQuery.isLoading ||
    getCurrenciesQuery.isLoading ||
    activeCurrenciesQuery.isLoading
  ) {
    return <InlineLoading />
  }

  const onSubmit = async (values: GeneralSettingsSchema) => {
    await updateTenantMut.mutateAsync({
      data: {
        tradeName: values.tradeName,
        slug: values.slug,
        reportingCurrency: values.reportingCurrency,
        name: values.tenantName,
        alertEvaluationIntervalSecs: Number(values.alertEvaluationInterval),
        keepLastEvents:
          values.keepLastEvents !== activeTenantData?.tenant?.keepLastEvents
            ? values.keepLastEvents
            : undefined,
      },
    })
  }

  const onSubmitCurrencies = async (values: z.infer<typeof currencySchema>) => {
    await updateTenantCurrencyMut.mutateAsync({
      currencies: values.selectedCurrencies,
    })
  }

  const currencyOptions = currencies.map(currency => ({
    label: (
      <span className="flex flex-row">
        <span className="pr-2">{currency.name}</span>
        <span>({currency.code})</span>
      </span>
    ),
    value: currency.code,
    keywords: [currency.name, currency.code],
  }))

  return (
    <div className="flex max-w-3xl flex-col gap-6">
      <Form {...methods}>
        <form onSubmit={methods.handleSubmit(onSubmit)}>
          <SettingsSection
            divided={false}
            title="Organization"
            description="Your company identity and where reporting figures roll up."
            action={
              <Button size="sm" disabled={!methods.formState.isDirty || updateTenantMut.isPending}>
                Save
              </Button>
            }
          >
            <div className="space-y-4">
              <SettingsFieldGrid>
                <InputFormField
                  name="tradeName"
                  label="Company trade name"
                  control={methods.control}
                  placeholder="ACME"
                  containerClassName="sm:col-span-2"
                />
                <InputFormField
                  name="tenantName"
                  control={methods.control}
                  label="Tenant name"
                  placeholder="Production"
                />
                <InputFormField
                  name="slug"
                  control={methods.control}
                  label="Tenant slug"
                  placeholder="prod"
                />
                <ComboboxFormField
                  name="reportingCurrency"
                  label="Reporting currency"
                  control={methods.control}
                  placeholder="Select"
                  hasSearch
                  description="Used across dashboards and reports."
                  options={currencyOptions}
                />
              </SettingsFieldGrid>

              <SettingsRowGroup>
                <SettingsRow
                  title="Customer emails"
                  description="Delivery, sender and email types are set in Customer emails."
                >
                  {activeTenantData?.tenant && (
                    <DeliveryModeBadge mode={activeTenantData.tenant.emailDeliveryMode} />
                  )}
                  <Button
                    type="button"
                    variant="outline"
                    size="sm"
                    onClick={() => navigate(`${basePath}/settings/emails`, { replace: true })}
                  >
                    Open
                  </Button>
                </SettingsRow>
                <Slot name="generalSettings.rows" control={methods.control} />
                {/* No longer offered: shown only to a tenant that has it, to turn it off. */}
                {activeTenantData?.tenant?.keepLastEvents && (
                  <SettingsRow
                    title="Keep the last copy of re-sent events"
                    description="An event re-sent with the same id replaces the earlier copy instead of being ignored. Applies to new events only."
                  >
                    <Switch
                      checked={keepLastEvents}
                      disabled={!keepLastEvents}
                      onCheckedChange={v =>
                        methods.setValue('keepLastEvents', v, { shouldDirty: true })
                      }
                    />
                  </SettingsRow>
                )}
              </SettingsRowGroup>
            </div>
          </SettingsSection>
        </form>
      </Form>

      <Form {...currencyMethods}>
        <form onSubmit={currencyMethods.handleSubmit(onSubmitCurrencies)}>
          <SettingsSection
            title="Currencies"
            description="Currencies customers can be billed in. Once linked to a customer, a currency can’t be removed."
            action={
              <Button
                size="sm"
                disabled={
                  currencyMethods.getValues().selectedCurrencies.length === 0 ||
                  updateTenantCurrencyMut.isPending
                }
              >
                Save
              </Button>
            }
          >
            <MultiSelectFormField
              name="selectedCurrencies"
              control={currencyMethods.control}
              placeholder="Select currencies"
              hasSearch
            >
              {currencyOptions.map((a, i) => (
                <MultiSelectItem key={'msc-' + i} value={a.value} keywords={a.keywords}>
                  {a.label}
                </MultiSelectItem>
              ))}
            </MultiSelectFormField>
          </SettingsSection>
        </form>
      </Form>
      <DangerZone />
    </div>
  )
}
