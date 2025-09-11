import {
  createConnectQueryKey,
  createProtobufSafeUpdater,
  useMutation,
} from '@connectrpc/connect-query'
import { Button, Card, Form, InputFormField, TextareaFormField } from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { useEffect } from 'react'
import { toast } from 'sonner'
import { z } from 'zod'

import { Loading } from '@/components/Loading'
import { InvoicingEntitySelect } from '@/features/settings/components/InvoicingEntitySelect'
import { useInvoicingEntity } from '@/features/settings/hooks/useInvoicingEntity'
import { useZodForm } from '@/hooks/useZodForm'
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
  const queryClient = useQueryClient()
  const { selectedEntityId: invoiceEntityId, isLoading, currentEntity } = useInvoicingEntity()

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

  const methods = useZodForm({
    schema: invoiceDetailsSchema,
  })

  useEffect(() => {
    if (currentEntity) {
      methods.setValue('invoiceNumberPattern', currentEntity.invoiceNumberPattern)
      methods.setValue('gracePeriodHours', currentEntity.gracePeriodHours)
      methods.setValue('netTerms', currentEntity.netTerms)
      methods.setValue('invoiceFooterInfo', currentEntity.invoiceFooterInfo)
      methods.setValue('invoiceFooterLegal', currentEntity.invoiceFooterLegal)
      methods.setValue('logoAttachmentId', currentEntity.logoAttachmentId)
      methods.setValue('brandColor', currentEntity.brandColor)
    } else {
      methods.reset()
    }
  }, [currentEntity])

  if (isLoading) {
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
                <InvoicingEntitySelect />
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
    </div>
  )
}
