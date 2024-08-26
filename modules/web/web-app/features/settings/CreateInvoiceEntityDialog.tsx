import {
  Button,
  ComboboxFormField,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  Form,
  InputFormField,
} from '@md/ui'
import { z } from 'zod'

import { getCountryFlagEmoji } from '@/features/settings/utils'
import { useZodForm } from '@/hooks/useZodForm'
import { useQuery } from '@/lib/connectrpc'
import { getCountries } from '@/rpc/api/instance/v1/instance-InstanceService_connectquery'
import {
  createInvoicingEntity,
  listInvoicingEntities,
} from '@/rpc/api/invoicingentities/v1/invoicingentities-InvoicingEntitiesService_connectquery'
import {
  createConnectQueryKey,
  createProtobufSafeUpdater,
  useMutation,
} from '@connectrpc/connect-query'
import { useQueryClient } from '@tanstack/react-query'
import { toast } from 'sonner'

const createInvoicingEntitySchema = z.object({
  legalName: z.string().min(1, 'Legal name is required'),
  country: z.string().min(1, 'Country is required'),
})

export const CreateInvoicingEntityDialog = ({
  open,
  setOpen,
  setInvoicingEntity,
}: {
  open: boolean
  setOpen: (open: boolean) => void
  setInvoicingEntity: (id: string) => void
}) => {
  const queryClient = useQueryClient()

  const createInvoicingEntityMut = useMutation(createInvoicingEntity, {
    onSuccess: async res => {
      if (res.entity) {
        queryClient.setQueryData(
          createConnectQueryKey(listInvoicingEntities),
          createProtobufSafeUpdater(listInvoicingEntities, prev => {
            return {
              entities: [...(prev?.entities ?? []), res.entity!],
            }
          })
        )
        toast.success('Invoicing entity created')
        setInvoicingEntity(res.entity.id)
        setOpen(false)
      }
    },
  })

  const getCountriesQuery = useQuery(getCountries)

  const methods = useZodForm({
    schema: createInvoicingEntitySchema,
  })

  const onSubmit = async (values: z.infer<typeof createInvoicingEntitySchema>) => {
    // TODO filter out if it hasn't changed
    await createInvoicingEntityMut.mutateAsync({
      data: {
        country: values.country,
        legalName: values.legalName,
      },
    })
  }

  return (
    <Dialog open={open} onOpenChange={setOpen}>
      <DialogContent className="sm:max-w-[425px]">
        <Form {...methods}>
          <form onSubmit={methods.handleSubmit(onSubmit)}>
            <DialogHeader>
              <DialogTitle>Create a new invoicing entity</DialogTitle>
              <DialogDescription>
                You will be able to associate different invoicing entities to different customers
              </DialogDescription>
            </DialogHeader>
            <div className="flex flex-col gap-4 py-4">
              <InputFormField
                name="legalName"
                control={methods.control}
                label="Legal name"
                placeholder="ACME Inc."
                containerClassName="col-span-4"
              />

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
            </div>
            <DialogFooter>
              <Button type="submit">Save changes</Button>
            </DialogFooter>
          </form>
        </Form>
      </DialogContent>
    </Dialog>
  )
}
