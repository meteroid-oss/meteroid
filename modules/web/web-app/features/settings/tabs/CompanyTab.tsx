import {
  createConnectQueryKey,
  createProtobufSafeUpdater,
  useMutation,
} from '@connectrpc/connect-query'
import {
  Badge,
  Button,
  Card,
  cn,
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
import { PlusIcon } from 'lucide-react'
import { useEffect, useState } from 'react'
import { useWatch } from 'react-hook-form'
import { toast } from 'sonner'
import { z } from 'zod'

import { Combobox } from '@/components/Combobox'
import { Loading } from '@/components/Loading'
import { CreateInvoicingEntityDialog } from '@/features/settings/CreateInvoiceEntityDialog'
import { getCountryFlagEmoji } from '@/features/settings/utils'
import { Methods, useZodForm } from '@/hooks/useZodForm'
import { useQuery } from '@/lib/connectrpc'
import { getCountries } from '@/rpc/api/instance/v1/instance-InstanceService_connectquery'
import {
  listInvoicingEntities,
  updateInvoicingEntity,
} from '@/rpc/api/invoicingentities/v1/invoicingentities-InvoicingEntitiesService_connectquery'

const invoicingEntitySchema = z.object({
  legalName: z.string().optional(),
  addressLine1: z.string().optional(),
  addressLine2: z.string().optional(),
  zipCode: z.string().optional(),
  state: z.string().optional(),
  city: z.string().optional(),
  vatNumber: z.string().optional(),
  country: z.string().optional(),
})

export const CompanyTab = () => {
  const listInvoicingEntitiesQuery = useQuery(listInvoicingEntities)
  const queryClient = useQueryClient()

  const [createDialogOpen, setCreateDialogOpen] = useState(false)

  const updateInvoicingEntityMut = useMutation(updateInvoicingEntity, {
    onSuccess: async res => {
      if (res.entity) {
        queryClient.setQueryData(
          createConnectQueryKey(listInvoicingEntities),
          createProtobufSafeUpdater(listInvoicingEntities, prev => {
            return {
              entities: prev?.entities.map(entity => {
                if (entity.id === res.entity?.id) {
                  return res.entity
                } else {
                  return entity
                }
              }),
            }
          })
        )
        toast.success('Invoicing entity updated')
      }
    },
  })

  const defaultInvoicingEntity = listInvoicingEntitiesQuery.data?.entities?.find(
    entity => entity.isDefault
  )

  const [invoiceEntityId, setInvoiceEntityId] = useState<string | undefined>(
    defaultInvoicingEntity?.id
  )

  const methods = useZodForm({
    schema: invoicingEntitySchema,
  })

  useEffect(() => {
    const entity = listInvoicingEntitiesQuery.data?.entities?.find(
      entity => entity.id === invoiceEntityId
    )
    console.log('useEffect', invoiceEntityId)

    if (entity) {
      console.log('entity', entity)
      methods.setValue('legalName', entity.legalName)
      methods.setValue('addressLine1', entity.addressLine1)
      methods.setValue('addressLine2', entity.addressLine2)
      methods.setValue('zipCode', entity.zipCode)
      methods.setValue('state', entity.state)
      methods.setValue('country', entity.country)
    } else {
      methods.reset()
    }
  }, [invoiceEntityId])

  useEffect(() => {
    if (defaultInvoicingEntity && !invoiceEntityId) {
      setInvoiceEntityId(defaultInvoicingEntity.id)
    }
  }, [defaultInvoicingEntity])

  if (listInvoicingEntitiesQuery.isLoading) {
    return <Loading />
  }

  const onSubmit = async (values: z.infer<typeof invoicingEntitySchema>) => {
    // TODO filter out if it hasn't changed
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
        zipCode: values.zipCode,
      },
    })
  }

  return (
    <div className="flex flex-col gap-4">
      <Form {...methods}>
        <form onSubmit={methods.handleSubmit(onSubmit)} className="space-y-4">
          <Card className="px-8 py-6 max-w-[950px]  space-y-4">
            <div className="grid grid-cols-6 gap-4  ">
              <div className="col-span-2">
                <h3 className="font-medium text-lg">Billing details</h3>
              </div>
              <div className="col-span-4 content-center  flex flex-row">
                <div className="flex-grow"></div>
                <Combobox
                  placeholder="Select"
                  className="max-w-[300px]"
                  value={invoiceEntityId}
                  onChange={setInvoiceEntityId}
                  options={
                    listInvoicingEntitiesQuery.data?.entities.map(entity => ({
                      label: (
                        <div className="flex flex-row w-full">
                          <div className="pr-2">{getCountryFlagEmoji(entity.country)}</div>
                          <div>{entity.legalName}</div>
                          <div className="flex-grow" />
                          {entity.isDefault && <Badge variant="primary">Default</Badge>}
                        </div>
                      ),
                      value: entity.id,
                    })) ?? []
                  }
                  action={
                    <Button
                      size="content"
                      variant="ghost"
                      hasIcon
                      className="w-full border-none h-full"
                      onClick={() => setCreateDialogOpen(true)}
                    >
                      <PlusIcon size="12" /> New invoicing entity
                    </Button>
                  }
                />
              </div>
            </div>
            <div className="grid grid-cols-6 gap-4 pt-1 ">
              <InputFormField
                name="legalName"
                label="Legal name"
                control={methods.control}
                placeholder="ACME Inc."
                containerClassName="col-span-6"
              />

              <InputFormField
                name="addressLine1"
                control={methods.control}
                label="Address line 1"
                placeholder="Line 1"
                containerClassName="col-span-3"
              />
              <InputFormField
                name="addressLine2"
                control={methods.control}
                label="Address line 2"
                placeholder="Line 2"
                containerClassName="col-span-3"
              />
              <CountrySelect className="col-span-2" methods={methods} />
              <InputFormField
                name="zipCode"
                control={methods.control}
                label="Zipcode"
                placeholder="ZIP"
                containerClassName="col-span-1"
              />
              <InputFormField
                name="state"
                control={methods.control}
                label="State"
                placeholder="State"
                containerClassName="col-span-1"
              />
              <InputFormField
                name="city"
                control={methods.control}
                label="City"
                placeholder="City"
                containerClassName="col-span-2"
              />

              <InputFormField
                name="vatNumber"
                label="VAT number / Tax ID"
                control={methods.control}
                placeholder="FRXXXXXXXXXXXXX"
                containerClassName="col-span-6"
              />
            </div>
            <div className="pt-4">
              <h3 className="font-medium text-lg">Accounting</h3>
            </div>
            <div className="grid grid-cols-6 gap-4 pt-1 ">
              <div className="col-span-4">
                <Label>Accounting currency</Label>
                <p className="text-xs text-muted-foreground">
                  The currency in which you will be accounting for your transactions. This is
                  derived from your billing country and cannot be modified once set, to be
                  compliant.
                </p>
              </div>
              <div className="col-span-1 "></div>
              <div className="col-span-1 content-center">
                <AccountingCurrencySelect methods={methods} />
              </div>
            </div>
            <div className="pt-10 flex justify-end items-center ">
              <div>
                <Button
                  size="sm"
                  disabled={
                    !methods.formState.isValid ||
                    !methods.formState.isDirty ||
                    updateInvoicingEntityMut.isPending
                  }
                >
                  Save changes
                </Button>
              </div>
            </div>
          </Card>
        </form>
      </Form>
      <CreateInvoicingEntityDialog
        open={createDialogOpen}
        setOpen={setCreateDialogOpen}
        setInvoicingEntity={setInvoiceEntityId}
      />
    </div>
  )
}

const AccountingCurrencySelect = ({
  methods,
}: {
  methods: Methods<typeof invoicingEntitySchema>
}) => {
  const country = useWatch({
    name: 'country',
    control: methods.control,
  })

  const getCountriesQuery = useQuery(getCountries)

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

const CountrySelect = ({
  methods,
  className,
}: {
  methods: Methods<typeof invoicingEntitySchema>
  className: string
}) => {
  const country = useWatch({
    name: 'country',
    control: methods.control,
  })

  const getCountriesQuery = useQuery(getCountries)

  const countryData = getCountriesQuery.data?.countries.find(c => c.code === country)

  return (
    <div className={cn('space-y-1', className)}>
      <Label>Country</Label>
      <Select value={country}>
        <SelectTrigger disabled={true}>
          <SelectValue placeholder="Select a country" />
        </SelectTrigger>
        <SelectContent hideWhenDetached>
          {countryData?.currency && (
            <SelectItem value={countryData.code}>
              {getCountryFlagEmoji(countryData.code)} {countryData.name}{' '}
            </SelectItem>
          )}
        </SelectContent>
      </Select>
    </div>
  )
}
