import {
  createConnectQueryKey,
  createProtobufSafeUpdater,
  useMutation,
} from '@connectrpc/connect-query'
import {
  Alert,
  AlertDescription,
  Button,
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
  ComboboxFormField,
  Form,
  InputFormField,
  Label,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { ChevronDown, ScaleIcon } from 'lucide-react'
import { useWatch } from 'react-hook-form'
import { useNavigate } from 'react-router-dom'
import { z } from 'zod'

import { Methods, useZodForm } from '@/hooks/useZodForm'
import { useQuery } from '@/lib/connectrpc'
import { schemas } from '@/lib/schemas'
import {
  getCountries,
  getInstance,
} from '@/rpc/api/instance/v1/instance-InstanceService_connectquery'
import { createOrganization } from '@/rpc/api/organizations/v1/organizations-OrganizationsService_connectquery'
import { me } from '@/rpc/api/users/v1/users-UsersService_connectquery'

export const OrganizationOnboarding: React.FC = () => {
  const getInstanceQuery = useQuery(getInstance)
  const meQuery = useQuery(me)

  const navigate = useNavigate()

  if (
    getInstanceQuery?.data?.instanceInitiated &&
    !getInstanceQuery.data.multiOrganizationEnabled &&
    meQuery.data
  ) {
    if (meQuery?.data?.organizations.length >= 1) {
      return (
        <div className="p-10">
          <div>
            You cannot create more organizations on this instance. Please contact your account
            manager.
          </div>
          <div>
            <Button onClick={() => navigate('/')}>Back</Button>
          </div>
        </div>
      )
    }

    return (
      <div className="p-10">
        <div>
          You don&apos;t have access to this instance. Request an invite link to your admin.
        </div>
        <div>
          <Button onClick={() => navigate('/logout')}>Logout</Button>
        </div>
      </div>
    )
  }

  return (
    <>
      <div className="md:w-[550px] w-full  px-6 py-12 sm:px-12 flex flex-col   ">
        <h2 className="text-xl font-semibold">Let&apos;s setup your organization</h2>
        <p className="mt-2 text-sm text-muted-foreground">Tell us more about your company</p>

        <div className="light h-full pt-4 ">
          <OrganizationOnboardingForm />
        </div>
      </div>
      <div className="grow hidden md:flex overflow:hidden">
        <img
          className="object-cover object-center w-full sm:rounded-lg"
          src="/img/auth.png"
          alt="Onboarding illustration"
        />
      </div>
    </>
  )
}

const OrganizationOnboardingForm = () => {
  const queryClient = useQueryClient()
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

  return (
    <Form {...methods}>
      <form onSubmit={methods.handleSubmit(onSubmit)} className="flex flex-col gap-2 h-full">
        <InputFormField
          name="tradeName"
          label="Company trade name"
          control={methods.control}
          placeholder="ACME"
        />

        <div className="grid grid-cols-2 gap-4">
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

          <div className="space-y-2">
            <Label>Accounting currency</Label>
            <AccountingCurrencySelect methods={methods} />
          </div>
        </div>

        <Alert>
          <ScaleIcon className="h-4 w-4 shrink-0 text-primary-foreground" />
          <AlertDescription className="text-muted-foreground text-xs">
            Your accounting currency is derived from your incorporation country. This does not
            affect your ability to bill in other currencies.
          </AlertDescription>
        </Alert>

        <div>
          <Collapsible>
            <CollapsibleTrigger className="text-sm font-medium pt-2 flex w-full">
              <div className="flex text-start items-center gap-2">
                <div className="text-sm ">Company details</div>
              </div>
              <div className="flex-grow"></div>
              <div className="flex text-start items-center gap-2">
                <div className="text-xs font-normal text-muted-foreground">(optional)</div>
                <ChevronDown className="h-4 w-4 shrink-0 text-muted-foreground transition-transform duration-200" />
              </div>
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
                  <Label>Company address</Label>
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

        <div className="flex-grow"></div>

        <Button variant="primary" type="submit" disabled={!methods.formState.isValid} className="">
          Create my organization
        </Button>
      </form>
    </Form>
  )
}

const AccountingCurrencySelect = ({
  methods,
}: {
  methods: Methods<typeof schemas.organizations.organizationOnboardingSchema>
}) => {
  const getCountriesQuery = useQuery(getCountries)
  const country = useWatch({
    name: 'country',
    control: methods.control,
  })

  const countryData = getCountriesQuery.data?.countries.find(c => c.code === country)

  return (
    <Select value={countryData?.currency}>
      <SelectTrigger disabled={true}>
        <SelectValue placeholder="Select a country" />
      </SelectTrigger>
      <SelectContent hideWhenDetached>
        {countryData?.currency && (
          <SelectItem value={countryData.currency}>{countryData.currency}</SelectItem>
        )}
      </SelectContent>
    </Select>
  )
}

const getCountryFlagEmoji = (countryCode: string) => {
  const codePoints = countryCode.split('').map(char => 127397 + char.charCodeAt(0))
  return String.fromCodePoint(...codePoints)
}
