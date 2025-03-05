import {
  createConnectQueryKey,
  createProtobufSafeUpdater,
  useMutation,
} from '@connectrpc/connect-query'
import { Badge, Button, Card, Form, InputFormField, TextareaFormField } from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { PlusIcon } from 'lucide-react'
import { useEffect, useState } from 'react'
import { toast } from 'sonner'
import { z } from 'zod'

import { Combobox } from '@/components/Combobox'
import { Loading } from '@/components/Loading'
import { CreateInvoicingEntityDialog } from '@/features/settings/CreateInvoiceEntityDialog'
import { getCountryFlagEmoji } from '@/features/settings/utils'
import { useZodForm } from '@/hooks/useZodForm'
import { useQuery } from '@/lib/connectrpc'
import {
  listInvoicingEntities,
  updateInvoicingEntity,
} from '@/rpc/api/invoicingentities/v1/invoicingentities-InvoicingEntitiesService_connectquery'

const invoiceDetailsSchema = z.object({
  invoiceNumberPattern: z.string().optional(),
  gracePeriodHours: z.number().optional(),
  netTerms: z.number().optional(),
  invoiceFooterInfo: z.string().optional(),
  invoiceFooterLegal: z.string().optional(),
  logoAttachmentId: z.string().optional(),
  brandColor: z.string().optional(),
})

export const InvoiceTab = () => {
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
    schema: invoiceDetailsSchema,
  })

  useEffect(() => {
    const entity = listInvoicingEntitiesQuery.data?.entities?.find(
      entity => entity.id === invoiceEntityId
    )

    if (entity) {
      console.log('entity', entity)
      methods.setValue('invoiceNumberPattern', entity.invoiceNumberPattern)
      methods.setValue('gracePeriodHours', entity.gracePeriodHours)
      methods.setValue('netTerms', entity.netTerms)
      methods.setValue('invoiceFooterInfo', entity.invoiceFooterInfo)
      methods.setValue('invoiceFooterLegal', entity.invoiceFooterLegal)
      methods.setValue('logoAttachmentId', entity.logoAttachmentId)
      methods.setValue('brandColor', entity.brandColor)
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

  const onSubmit = async (values: z.infer<typeof invoiceDetailsSchema>) => {
    // TODO filter out if it hasn't changed
    await updateInvoicingEntityMut.mutateAsync({
      id: invoiceEntityId,
      data: {
        brandColor: values.brandColor,
        gracePeriodHours: values.gracePeriodHours,
        invoiceFooterInfo: values.invoiceFooterInfo,
        invoiceFooterLegal: values.invoiceFooterLegal,
        invoiceNumberPattern: values.invoiceNumberPattern,
        logoAttachmentId: values.logoAttachmentId,
        netTerms: values.netTerms,
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
                <h3 className="font-medium text-lg">Invoice settings</h3>
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
                          {entity.isDefault && (
                            <Badge variant="primary" size={'sm'}>
                              Default
                            </Badge>
                          )}
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
                name="invoiceNumberPattern"
                label="Invoice number pattern"
                control={methods.control}
                placeholder="ACME Inc."
                containerClassName="col-span-6"
                description="Use the following placeholders: {number} - mandatory sequential number, {YYYY} - year, {MM} - month, {DD} - day"
              />

              <InputFormField
                name="gracePeriodHours"
                control={methods.control}
                label="Grace period (hours)"
                placeholder="24"
                type="number"
                containerClassName="col-span-3"
              />
              <InputFormField
                name="netTerms"
                control={methods.control}
                label="Net terms (days)"
                type="number"
                placeholder="30"
                containerClassName="col-span-3"
              />
            </div>
            <div className="pt-4">
              <h3 className="font-medium text-lg">Invoice footer</h3>
            </div>
            <div className="grid grid-cols-6 gap-4 pt-1 ">
              <TextareaFormField
                name="invoiceFooterInfo"
                control={methods.control}
                label="Additional information"
                containerClassName="col-span-6"
              />
              <TextareaFormField
                name="invoiceFooterLegal"
                label="Legal information"
                control={methods.control}
                containerClassName="col-span-6"
              />
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
