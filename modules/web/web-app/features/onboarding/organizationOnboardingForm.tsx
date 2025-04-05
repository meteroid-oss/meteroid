import {
  createConnectQueryKey,
  createProtobufSafeUpdater,
  useMutation,
} from '@connectrpc/connect-query'
import {
  Button,
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
  ComboboxFormField,
  Flex,
  Form,
  InputFormField,
  Label,
  Separator,
} from '@md/ui'
import { ArrowLeft, ChevronDown } from 'lucide-react'
import { useNavigate } from 'react-router-dom'
import { z } from 'zod'

import { AccountingCurrencySelect } from '@/features/onboarding/accountingCurrencySelect'
import { convertEnterpriseName } from '@/features/onboarding/utils'
import { getCountryFlagEmoji } from '@/features/settings/utils'
import { useZodForm } from '@/hooks/useZodForm'
import { useQuery } from '@/lib/connectrpc'
import { queryClient } from '@/lib/react-query'
import { schemas } from '@/lib/schemas'
import {
  getCountries,
  getInstance,
} from '@/rpc/api/instance/v1/instance-InstanceService_connectquery'
import { createOrganization } from '@/rpc/api/organizations/v1/organizations-OrganizationsService_connectquery'
import { me } from '@/rpc/api/users/v1/users-UsersService_connectquery'

export const OrganizationOnboardingForm = () => {
  const getCountriesQuery = useQuery(getCountries)

  const navigate = useNavigate()

  const methods = useZodForm({
    schema: schemas.organizations.organizationOnboardingSchema,
    defaultValues: {},
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

        queryClient.invalidateQueries({ queryKey: createConnectQueryKey(getInstance) })

        navigate('/' + res.organization.slug)
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

  const tradeName = methods.watch('tradeName')

  return (
    <Flex direction="column" className="w-full h-full gap-2 p-[40px] lg:p-[52px]">
      <Button
        variant="secondary"
        onClick={() => navigate('/onboarding/user')}
        className="text-muted-foreground rounded-2xl w-12 h-[28px] mb-2"
      >
        <ArrowLeft size={15} />
      </Button>
      <div className="font-medium text-xl -mb-0.5">Create your workspace</div>
      <div className="text-muted-foreground text-[13px] mb-5 leading-[18px]">
        Tell us about your company. We’ll use this information to setup your billing journey!
      </div>
      <Form {...methods}>
        <form onSubmit={methods.handleSubmit(onSubmit)} className="flex flex-col gap-2 h-full">
          <Flex direction="column" className="gap-1.5">
            <InputFormField
              name="tradeName"
              label="Company trade name"
              control={methods.control}
              placeholder="Acme inc"
            />
            <div className="text-xs text-muted-foreground/50">
              app.meteroid.com/{convertEnterpriseName(tradeName)}
            </div>
          </Flex>

          <div className="grid grid-cols-1 lg:grid-cols-2 gap-4 mt-5">
            <ComboboxFormField
              name="country"
              label="Incorporation country"
              control={methods.control}
              placeholder="Select"
              hasSearch
              options={
                getCountriesQuery.data?.countries.map(country => ({
                  label: (
                    <span className="flex flex-row">
                      <span className="pr-2">{getCountryFlagEmoji(country.code)}</span>
                      <span>{country.name}</span>
                    </span>
                  ),
                  value: country.code,
                  keywords: [country.name, country.code],
                })) ?? []
              }
            />

            <div className="space-y-1">
              <Label className="text-muted-foreground">Accounting currency</Label>
              <AccountingCurrencySelect methods={methods} />
            </div>
          </div>

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
