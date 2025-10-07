import {
  createConnectQueryKey,
  createProtobufSafeUpdater,
  useMutation,
} from '@connectrpc/connect-query'
import {
  Button,
  Card,
  cn,
  Form,
  Input,
  InputFormField,
  Label,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { RefreshCwIcon, UploadIcon, XIcon } from 'lucide-react'
import { useEffect, useState } from 'react'
import { useWatch } from 'react-hook-form'
import { toast } from 'sonner'
import { z } from 'zod'

import { Loading } from '@/components/Loading'
import { InvoicingEntitySelect } from '@/features/settings/components/InvoicingEntitySelect'
import { useInvoicingEntity } from '@/features/settings/hooks/useInvoicingEntity'
import { getCountryFlagEmoji, getCountryName } from '@/features/settings/utils'
import { Methods, useZodForm } from '@/hooks/useZodForm'
import { useQuery } from '@/lib/connectrpc'
import { env } from '@/lib/env'
import { getCountries } from '@/rpc/api/instance/v1/instance-InstanceService_connectquery'
import {
  getInvoicingEntity,
  listInvoicingEntities,
  updateInvoicingEntity,
  uploadInvoicingEntityLogo,
} from '@/rpc/api/invoicingentities/v1/invoicingentities-InvoicingEntitiesService_connectquery'
import { InvoicingEntity } from '@/rpc/api/invoicingentities/v1/models_pb'

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
  const queryClient = useQueryClient()
  const {
    selectedEntityId: invoiceEntityId,
    isLoading,
    currentEntity: currentInvoicingEntity,
  } = useInvoicingEntity()

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
        queryClient.invalidateQueries({
          queryKey: createConnectQueryKey(getInvoicingEntity, { id: res.entity?.id }),
        })
        toast.success('Invoicing entity updated')
      }
    },
  })

  const methods = useZodForm({
    schema: invoicingEntitySchema,
  })

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
      })
    } else {
      methods.reset()
    }
  }, [currentInvoicingEntity])

  if (isLoading) {
    return <Loading/>
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
                <InvoicingEntitySelect/>
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
              <CountrySelect className="col-span-2" methods={methods}/>
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
              <h3 className="font-medium text-lg">Brand</h3>
            </div>
            <div className="grid grid-cols-6 gap-4 pt-1 ">
              <div className="col-span-4">
                <Label>Logo</Label>
                <p className="text-xs text-muted-foreground">
                  Used in invoices, emails and portals. Max size 1MB, PNG or JPEG.
                </p>
              </div>
              <div className="col-span-1 "></div>
              <div className="col-span-1 content-end">
                {currentInvoicingEntity && <FileUpload entity={currentInvoicingEntity}/>}
              </div>
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
                <AccountingCurrencySelect methods={methods}/>
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
    </div>
  )
}

const FileUpload = ({ entity }: { entity: InvoicingEntity }) => {
  const [isUploading, setIsUploading] = useState(false)
  const queryClient = useQueryClient()

  const updateInvoicingEntityMut = useMutation(uploadInvoicingEntityLogo, {
    onSuccess: async res => {
      queryClient.setQueryData(
        createConnectQueryKey(getInvoicingEntity, { id: entity.id }),
        createProtobufSafeUpdater(getInvoicingEntity, prev => {
          return {
            entity: {
              ...prev?.entity,
              logoAttachmentId: res.logoUid,
            },
          }
        })
      )
      toast.success('Invoicing entity updated')
    },
  })

  const handleFileUpload = async (event: React.ChangeEvent<HTMLInputElement>) => {
    const file = event.target.files?.[0]
    if (!file) return

    setIsUploading(true)

    const reader = new FileReader()
    reader.onload = async () => {
      const arrayBuffer = reader.result as ArrayBuffer
      const uint8Array = new Uint8Array(arrayBuffer)

      try {
        await updateInvoicingEntityMut.mutateAsync({
          id: entity.id,
          file: {
            data: uint8Array,
          },
        })
      } catch (error) {
        console.error('Error uploading file:', error)
        toast.error('Failed to upload file. Please try again.')
      } finally {
        setIsUploading(false)
      }
    }

    reader.readAsArrayBuffer(file)
  }

  const clearFile = async () => {
    setIsUploading(true)
    try {
      await updateInvoicingEntityMut.mutateAsync({
        id: entity.id,
        file: undefined,
      })
    } catch (error) {
      toast.error('Failed to update logo.')
    } finally {
      setIsUploading(false)
    }
  }

  return (
    <div className="flex flex-row space-y-2 space-x-2 w-full">
      <div className="flex-1 ">
        {entity.logoAttachmentId && (
          <div className="relative w-12 h-12 rounded overflow-hidden group float-end">
            <img
              src={env.meteroidRestApiUri + '/files/v1/logo/' + entity.logoAttachmentId}
              alt="Uploaded file"
              className="w-full h-full object-cover"
            />
            <Button
              size="icon"
              variant="destructive"
              type="button"
              className="absolute top-0 right-0 w-6 h-6 p-1 opacity-0 group-hover:opacity-100 transition-opacity"
              onClick={clearFile}
            >
              <XIcon size="16"/>
            </Button>
          </div>
        )}
      </div>
      <Input
        type="file"
        id="file-upload"
        className="hidden"
        accept="image/*"
        onChange={handleFileUpload}
      />
      <Button size="icon" variant="ghost" type="button" className="self-end">
        <Label htmlFor="file-upload" className="cursor-pointer w-full h-full flex">
          {isUploading ? (
            <RefreshCwIcon size="16" className="animate-spin m-auto"/>
          ) : (
            <UploadIcon size="16" className="m-auto"/>
          )}
        </Label>
      </Button>
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
        <SelectValue placeholder="Select a country"/>
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
          <SelectValue placeholder="Select a country"/>
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
