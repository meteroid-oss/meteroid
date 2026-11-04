import { useMutation } from '@connectrpc/connect-query'
import { Button, Flex, Form, InputFormField, Label, Spinner } from '@md/ui'
import { ArrowLeft } from 'lucide-react'
import { useWatch } from 'react-hook-form'
import { useNavigate } from 'react-router-dom'
import { z } from 'zod'

import { CountrySelect } from '@/components/CountrySelect'
import {
  INVOICE_LANGUAGE_DEFAULT,
  INVOICE_LANGUAGE_OPTIONS,
  InvoiceLanguageSelect,
  invoiceLanguageDeservesExplicitChoice,
  invoiceLanguageForCreate,
} from '@/components/InvoiceLanguageSelect'
import { AccountingCurrencySelect } from '@/features/onboarding/accountingCurrencySelect'
import { useIsCloud } from '@/hooks/useIsCloud'
import { buildSyncParam } from '@/hooks/useSyncQueries'
import { useZodForm } from '@/hooks/useZodForm'
import { schemas } from '@/lib/schemas'
import { createOrganization } from '@/rpc/api/organizations/v1/organizations-OrganizationsService_connectquery'
import { TenantEnvironmentEnum } from '@/rpc/api/tenants/v1/models_pb'

const getBrowserLocale = (): string | undefined => navigator.language || navigator.languages?.[0]

const getBrowserCountryCode = (): string | undefined => {
  const locale = getBrowserLocale()
  if (!locale) return undefined
  // Extract country from locale like "en-US" -> "US", "fr-FR" -> "FR"
  const parts = locale.split('-')
  return parts.length > 1 ? parts[1].toUpperCase() : undefined
}

/**
 * The language half of the browser locale, which the country derivation above discards.
 *
 * It is the best signal available at signup for the countries where the country alone
 * does not settle the language: a Walloon arriving with `fr-BE` has already told us they
 * work in French, and defaulting them to Dutch — which `countries.json` does for every
 * Belgian entity — is exactly the guess Belgian language law punishes. Only a seed: the
 * operator sees it in the field and can change it before submitting.
 */
const getBrowserLanguageDefault = (): string | undefined => {
  const locale = getBrowserLocale()
  if (!locale) return undefined
  const language = locale.split('-')[0]?.toLowerCase()
  return INVOICE_LANGUAGE_OPTIONS.find(
    option => option.value !== INVOICE_LANGUAGE_DEFAULT && option.value.startsWith(`${language}-`)
  )?.value
}

export const OrganizationOnboardingForm = () => {
  const navigate = useNavigate()
  const isCloud = useIsCloud()

  const methods = useZodForm({
    schema: schemas.organizations.organizationOnboardingSchema,
    defaultValues: {
      country: getBrowserCountryCode(),
      invoicingLanguage: getBrowserLanguageDefault() ?? INVOICE_LANGUAGE_DEFAULT,
    },
    mode: 'onSubmit',
  })

  const country = useWatch({ control: methods.control, name: 'country' })
  const askForLanguage = invoiceLanguageDeservesExplicitChoice(country)

  const createOrganizationMut = useMutation(createOrganization, {
    onSuccess: async res => {
      if (res.organization) {
        // Self-hosted signup seeds no sandbox: the demo data is offered as a last step.
        const offerDemoData =
          isCloud === false &&
          !res.organization.tenants.some(t => t.environment === TenantEnvironmentEnum.SANDBOX)
        // Full page navigation to clear React Query cache — org-scoped queries
        // (like listTenants) use the x-md-context header for context, not request params,
        // so their cache keys are identical across orgs.
        window.location.href = offerDemoData
          ? `/onboarding/demo/${res.organization.slug}`
          : `/${res.organization.slug}?${buildSyncParam('stats')}`
      }
    },
  })

  const onSubmit = async (
    data: z.infer<typeof schemas.organizations.organizationOnboardingSchema>
  ) => {
    await createOrganizationMut.mutateAsync({
      country: data.country,
      tradeName: data.tradeName,
      invoicingLanguage: invoiceLanguageDeservesExplicitChoice(data.country)
        ? invoiceLanguageForCreate(data.invoicingLanguage)
        : undefined,
    })
  }

  return (
    <Flex
      direction="column"
      className="w-full h-full gap-2 p-6 sm:p-8 md:p-10 lg:p-[52px] overflow-y-auto"
    >
      <Button
        variant="secondary"
        onClick={() => navigate(-1)}
        className="text-muted-foreground rounded-2xl w-12 h-[28px] mb-2"
      >
        <ArrowLeft size={15} />
      </Button>
      <div className="font-medium text-xl -mb-0.5">Create your workspace</div>
      <div className="text-muted-foreground text-[13px] mb-5 leading-[18px]">
        Tell us about your company. We’ll use this information to setup your billing journey!
      </div>
      <Form {...methods}>
        <form onSubmit={methods.handleSubmit(onSubmit)} className="flex flex-col gap-2 min-h-0">
          <Flex direction="column" className="gap-1.5">
            <InputFormField
              name="tradeName"
              label="Company trade name"
              control={methods.control}
              placeholder="Acme inc"
            />
            {/*<div className="text-xs text-muted-foreground/50">*/}
            {/*  app.meteroid.com/{convertEnterpriseName(tradeName)}*/}
            {/*</div>*/}
          </Flex>

          <div className="grid grid-cols-1 lg:grid-cols-2 gap-4 mt-5 mb-5">
            <CountrySelect
              name="country"
              label="Incorporation country"
              control={methods.control}
              placeholder="Select"
            />

            <div className="space-y-1">
              <Label className="text-muted-foreground">Accounting currency</Label>
              <AccountingCurrencySelect methods={methods} />
            </div>

            {/* Only where the country cannot settle it. Seeded from the browser locale,
                which is usually right and always changeable. */}
            {askForLanguage && (
              <div className="space-y-1.5 lg:col-span-2">
                <InvoiceLanguageSelect
                  name="invoicingLanguage"
                  label="Invoice language"
                  control={methods.control}
                />
              </div>
            )}
          </div>

          <Button
            variant="primary"
            type="submit"
            hasIcon={createOrganizationMut.isPending}
            disabled={!methods.formState.isValid || createOrganizationMut.isPending}
            className="mt-auto"
          >
            {createOrganizationMut.isPending && <Spinner />}
            Create my organization
          </Button>
        </form>
      </Form>
    </Flex>
  )
}
