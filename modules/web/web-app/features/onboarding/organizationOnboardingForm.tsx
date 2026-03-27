import { useMutation } from '@connectrpc/connect-query'
import { Button, Flex, Form, InputFormField, Label } from '@md/ui'
import { ArrowLeft } from 'lucide-react'
import { useNavigate } from 'react-router-dom'
import { z } from 'zod'

import { CountrySelect } from '@/components/CountrySelect'
import { AccountingCurrencySelect } from '@/features/onboarding/accountingCurrencySelect'
import { buildSyncParam } from '@/hooks/useSyncQueries'
import { useZodForm } from '@/hooks/useZodForm'
import { schemas } from '@/lib/schemas'
import { createOrganization } from '@/rpc/api/organizations/v1/organizations-OrganizationsService_connectquery'

const getBrowserCountryCode = (): string | undefined => {
  const locale = navigator.language || navigator.languages?.[0]
  if (!locale) return undefined
  // Extract country from locale like "en-US" -> "US", "fr-FR" -> "FR"
  const parts = locale.split('-')
  return parts.length > 1 ? parts[1].toUpperCase() : undefined
}

export const OrganizationOnboardingForm = () => {
  const navigate = useNavigate()

  const methods = useZodForm({
    schema: schemas.organizations.organizationOnboardingSchema,
    defaultValues: {
      country: getBrowserCountryCode(),
    },
    mode: 'onSubmit',
  })

  const createOrganizationMut = useMutation(createOrganization, {
    onSuccess: async res => {
      if (res.organization) {
        // Full page navigation to clear React Query cache — org-scoped queries
        // (like listTenants) use the x-md-context header for context, not request params,
        // so their cache keys are identical across orgs.
        window.location.href = `/${res.organization.slug}?${buildSyncParam('stats')}`
      }
    },
  })

  const onSubmit = async (
    data: z.infer<typeof schemas.organizations.organizationOnboardingSchema>
  ) => {
    await createOrganizationMut.mutateAsync({
      country: data.country,
      tradeName: data.tradeName,
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
          </div>

          <Button
            variant="primary"
            type="submit"
            disabled={!methods.formState.isValid}
            className="mt-auto"
          >
            Create my organization
          </Button>
        </form>
      </Form>
    </Flex>
  )
}
