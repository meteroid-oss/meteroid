import {
  createConnectQueryKey,
  createProtobufSafeUpdater,
  useMutation,
} from '@connectrpc/connect-query'
import { Button, Flex, Form, InputFormField, Label } from '@md/ui'
import { ArrowLeft } from 'lucide-react'
import { useNavigate } from 'react-router-dom'
import { z } from 'zod'

import { CountrySelect } from '@/components/CountrySelect'
import { AccountingCurrencySelect } from '@/features/onboarding/accountingCurrencySelect'
import { useZodForm } from '@/hooks/useZodForm'
import { queryClient } from '@/lib/react-query'
import { schemas } from '@/lib/schemas'
import { getInstance } from '@/rpc/api/instance/v1/instance-InstanceService_connectquery'
import {
  createOrganization,
  getCurrentOrganizations,
} from '@/rpc/api/organizations/v1/organizations-OrganizationsService_connectquery'
import { me } from '@/rpc/api/users/v1/users-UsersService_connectquery'

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
        queryClient.setQueryData(
          createConnectQueryKey(me),
          createProtobufSafeUpdater(me, prev => {
            return {
              ...prev,
              organizations: [...(prev?.organizations ?? []), res.organization!],
            }
          })
        )

        await queryClient.invalidateQueries({ queryKey: createConnectQueryKey(getInstance) })
        await queryClient.invalidateQueries({
          queryKey: createConnectQueryKey(getCurrentOrganizations),
        })

        navigate('/' + res.organization.slug + '?just_onboarded=true')
      }
    },
  })

  const onSubmit = async (
    data: z.infer<typeof schemas.organizations.organizationOnboardingSchema>
  ) => {
    await createOrganizationMut.mutateAsync({
      country: data.country,
      tradeName: data.tradeName,
      legalName: data.legalName,
      addressLine1: data.addressLine1,
      addressLine2: data.addressLine2,
      city: data.city,
      state: data.state,
      zipCode: data.zipCode,
      vatNumber: data.vatNumber,
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

          {/* Company details section commented out - data is not saved by backend yet
          <div>
            <Collapsible>
              <CollapsibleTrigger className="text-sm font-medium pt-2 flex w-full">
                <Flex direction="column" className="w-full">
                  <Separator className="mt-7 mb-6 h-[0.5px]" />
                  <Flex justify="between" align="center">
                    <div className="text-sm ">Company details</div>
                    <ChevronDown className="h-4 w-4 shrink-0 text-muted-foreground transition-transform duration-200" />
                  </Flex>
                  <Separator className="mt-6 mb-3.5 h-[0.5px]" />
                </Flex>
              </CollapsibleTrigger>
              <CollapsibleContent>
                <span className="text-sm text-muted-foreground">
                  You will need to configure this before emitting invoices
                </span>

                <div className="grid grid-cols-6 gap-2 pt-1 ">
                  <InputFormField
                    name="legalName"
                    label="Legal name"
                    control={methods.control}
                    placeholder="ACME Inc."
                    containerClassName="col-span-3"
                  />
                  <InputFormField
                    name="vatNumber"
                    label="VAT number / Tax ID"
                    control={methods.control}
                    placeholder="FRXXXXXXXXXXXXX"
                    containerClassName="col-span-3"
                  />

                  <div className="col-span-6">
                    <Label className="text-muted-foreground">Company address</Label>
                  </div>
                  <InputFormField
                    name="addressLine1"
                    control={methods.control}
                    placeholder="Line 1"
                    containerClassName="col-span-3"
                  />
                  <InputFormField
                    name="addressLine2"
                    control={methods.control}
                    placeholder="Line 2"
                    containerClassName="col-span-3"
                  />
                  <InputFormField
                    name="zipCode"
                    control={methods.control}
                    placeholder="ZIP"
                    containerClassName="col-span-1"
                  />
                  <InputFormField
                    name="state"
                    control={methods.control}
                    placeholder="State"
                    containerClassName="col-span-2"
                  />
                  <InputFormField
                    name="city"
                    control={methods.control}
                    placeholder="City"
                    containerClassName="col-span-3"
                  />
                </div>
              </CollapsibleContent>
            </Collapsible>
          </div>
          */}

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
