import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query'
import {
  Button,
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
import { useEffect } from 'react'
import { useWatch } from 'react-hook-form'
import { toast } from 'sonner'
import { z } from 'zod'

import { CopyToClipboardButton } from '@/components/CopyToClipboard'
import { InlineLoading } from '@/components/Loading'
import { InvoicingEntitySelect } from '@/features/settings/components/InvoicingEntitySelect'
import {
  SettingsFieldGrid,
  SettingsRow,
  SettingsRowGroup,
  SettingsSection,
} from '@/features/settings/components/SettingsSection'
import { useInvoicingEntity } from '@/features/settings/hooks/useInvoicingEntity'
import { getCountryFlagEmoji, getCountryName } from '@/features/settings/utils'
import { Methods, useZodForm } from '@/hooks/useZodForm'
import { useQuery } from '@/lib/connectrpc'
import { getCountries } from '@/rpc/api/instance/v1/instance-InstanceService_connectquery'
import {
  getInvoicingEntity,
  listInvoicingEntities,
  updateInvoicingEntity,
} from '@/rpc/api/invoicingentities/v1/invoicingentities-InvoicingEntitiesService_connectquery'

const merchantSchema = z.object({
  legalName: z.string().optional(),
  addressLine1: z.string().optional(),
  addressLine2: z.string().optional(),
  zipCode: z.string().optional(),
  state: z.string().optional(),
  city: z.string().optional(),
  vatNumber: z.string().optional(),
  legalNumber: z.string().optional(),
  country: z.string().optional(),
  contactName: z.string().optional(),
  contactPhone: z.string().optional(),
  contactEmail: z.string().email().optional().or(z.literal('')),
})

export const CompanyTab = () => {
  const queryClient = useQueryClient()
  const {
    selectedEntityId: invoiceEntityId,
    isLoading,
    currentEntity: currentInvoicingEntity,
  } = useInvoicingEntity()

  const updateInvoicingEntityMut = useMutation(updateInvoicingEntity, {
    onSuccess: async res => {
      if (res.entity) {
        queryClient.invalidateQueries({
          queryKey: createConnectQueryKey({
            schema: listInvoicingEntities,
            cardinality: undefined,
          }),
        })
        queryClient.invalidateQueries({
          queryKey: createConnectQueryKey({
            schema: getInvoicingEntity,
            input: { id: res.entity?.id },
            cardinality: 'finite',
          }),
        })
        toast.success('Invoicing entity updated')
      }
    },
  })

  const methods = useZodForm({ schema: merchantSchema })

  useEffect(() => {
    if (currentInvoicingEntity) {
      methods.reset({
        legalName: currentInvoicingEntity.legalName || '',
        addressLine1: currentInvoicingEntity.addressLine1 || '',
        addressLine2: currentInvoicingEntity.addressLine2 || '',
        zipCode: currentInvoicingEntity.zipCode || '',
        state: currentInvoicingEntity.state || '',
        country: currentInvoicingEntity.country || '',
        city: currentInvoicingEntity.city || '',
        vatNumber: currentInvoicingEntity.vatNumber || '',
        legalNumber: currentInvoicingEntity.legalNumber || '',
        contactName: currentInvoicingEntity.contactName || '',
        contactPhone: currentInvoicingEntity.contactPhone || '',
        contactEmail: currentInvoicingEntity.contactEmail || '',
      })
    } else {
      methods.reset()
    }
  }, [currentInvoicingEntity])

  if (isLoading) {
    return <InlineLoading />
  }

  const onSubmit = async (values: z.infer<typeof merchantSchema>) => {
    await updateInvoicingEntityMut.mutateAsync({
      id: invoiceEntityId,
      data: {
        addressLine1: values.addressLine1,
        addressLine2: values.addressLine2,
        city: values.city,
        country: values.country,
        legalName: values.legalName,
        state: values.state,
        vatNumber: values.vatNumber,
        legalNumber: values.legalNumber,
        zipCode: values.zipCode,
        contactName: values.contactName,
        contactPhone: values.contactPhone,
        contactEmail: values.contactEmail,
      },
    })
  }

  return (
    <Form {...methods}>
      <form
        onSubmit={methods.handleSubmit(onSubmit)}
        autoComplete="off"
        className="flex max-w-3xl flex-col gap-6"
      >
        <SettingsSection
          divided={false}
          title="Billing details"
          description="Legal entity shown on invoices and used for tax."
          action={
            <div className="flex items-center gap-2">
              <InvoicingEntitySelect />
              <Button
                size="sm"
                disabled={!methods.formState.isDirty || updateInvoicingEntityMut.isPending}
              >
                Save
              </Button>
            </div>
          }
        >
          <SettingsFieldGrid>
            <InputFormField
              name="legalName"
              label="Legal name"
              control={methods.control}
              placeholder="ACME Inc."
              autoComplete="off"
            />
            <div className="space-y-1">
              <Label>Invoice entity ID</Label>
              <CopyToClipboardButton
                text={invoiceEntityId || ''}
                buttonClassName="w-full justify-start"
                buttonVariant="outline"
              />
            </div>
            <InputFormField
              name="addressLine1"
              control={methods.control}
              label="Address line 1"
              placeholder="Line 1"
              autoComplete="off"
            />
            <InputFormField
              name="addressLine2"
              control={methods.control}
              label="Address line 2"
              placeholder="Line 2"
              autoComplete="off"
            />
            <CountrySelect methods={methods} />
            <InputFormField
              name="zipCode"
              control={methods.control}
              label="Zip code"
              placeholder="ZIP"
              autoComplete="off"
            />
            <InputFormField
              name="state"
              control={methods.control}
              label="State"
              placeholder="State"
              autoComplete="off"
            />
            <InputFormField
              name="city"
              control={methods.control}
              label="City"
              placeholder="City"
              autoComplete="off"
            />
            <InputFormField
              name="vatNumber"
              label="VAT number / Tax ID"
              control={methods.control}
              placeholder="FRXXXXXXXXXXXXX"
              autoComplete="off"
            />
            <InputFormField
              name="legalNumber"
              label="National company registration number"
              control={methods.control}
              placeholder="SIREN / SIRET, HRB, ..."
              autoComplete="off"
            />
          </SettingsFieldGrid>
        </SettingsSection>

        <SettingsSection
          title="Invoice contact"
          description="Who your customers reach about an invoice. The email is printed on invoices."
        >
          <SettingsFieldGrid>
            <InputFormField
              name="contactName"
              label="Contact name"
              control={methods.control}
              placeholder="Accounts receivable"
              autoComplete="off"
            />
            <InputFormField
              name="contactPhone"
              label="Phone"
              control={methods.control}
              placeholder="+49 30 1234567"
              autoComplete="off"
            />
            <InputFormField
              name="contactEmail"
              label="Email"
              control={methods.control}
              placeholder="billing@example.com"
              autoComplete="off"
            />
          </SettingsFieldGrid>
        </SettingsSection>

        <SettingsSection title="Accounting">
          <SettingsRowGroup>
            <SettingsRow
              title="Accounting currency"
              description="Derived from your billing country. Fixed once set, to stay compliant."
            >
              <AccountingCurrencySelect methods={methods} />
            </SettingsRow>
          </SettingsRowGroup>
        </SettingsSection>
      </form>
    </Form>
  )
}

const AccountingCurrencySelect = ({ methods }: { methods: Methods<typeof merchantSchema> }) => {
  const country = useWatch({ name: 'country', control: methods.control })
  const getCountriesQuery = useQuery(getCountries)
  const countryData = getCountriesQuery.data?.countries.find(c => c.code === country)

  return (
    <Select value={countryData?.currency}>
      <SelectTrigger disabled className="w-[160px]">
        <SelectValue placeholder="—" />
      </SelectTrigger>
      <SelectContent hideWhenDetached>
        {countryData?.currency && (
          <SelectItem value={countryData.currency}>{countryData.currency}</SelectItem>
        )}
      </SelectContent>
    </Select>
  )
}

const CountrySelect = ({ methods }: { methods: Methods<typeof merchantSchema> }) => {
  const country = useWatch({ name: 'country', control: methods.control })
  const getCountriesQuery = useQuery(getCountries)
  const countryData = getCountriesQuery.data?.countries.find(c => c.code === country)

  return (
    <div className="space-y-1">
      <Label>Country</Label>
      <Select value={country}>
        <SelectTrigger disabled>
          <SelectValue placeholder="Select a country" />
        </SelectTrigger>
        <SelectContent hideWhenDetached>
          {countryData?.currency && (
            <SelectItem value={countryData.code}>
              {getCountryFlagEmoji(countryData.code)} {getCountryName(countryData.code)}{' '}
            </SelectItem>
          )}
        </SelectContent>
      </Select>
    </div>
  )
}
