import { useMutation } from '@connectrpc/connect-query'
import {
  Button,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  Form,
  InputFormField,
} from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { toast } from 'sonner'
import { z } from 'zod'

import { useZodForm } from '@/hooks/useZodForm'
import { rpcErrorMessage } from '@/lib/rpcError'
import { updateCreditUnit } from '@/rpc/api/creditunits/v1/creditunits-CreditUnitsService_connectquery'

import { invalidateCreditUnits } from './queries'

import type { CreditUnit } from '@/rpc/api/creditunits/v1/models_pb'

// The store bounds labels in bytes once trimmed (`domain/credit_units.rs`).
const byteLength = (value: string) => new TextEncoder().encode(value.trim()).length

const label = (field: string) =>
  z
    .string()
    .refine(value => value.trim().length > 0, `${field} is required`)
    .refine(value => byteLength(value) <= 64, `${field} must be at most 64 characters`)

export const editCreditUnitSchema = z.object({
  name: label('Name'),
  pluralName: label('Plural name'),
  symbol: z.string().refine(value => byteLength(value) <= 8, 'At most 8 characters'),
})

type Values = z.infer<typeof editCreditUnitSchema>

export const EditCreditUnitDialog = ({
  unit,
  onClose,
}: {
  unit: CreditUnit
  onClose: () => void
}) => {
  const queryClient = useQueryClient()

  const methods = useZodForm({
    schema: editCreditUnitSchema,
    defaultValues: {
      name: unit.name,
      pluralName: unit.pluralName,
      symbol: unit.symbol ?? '',
    },
  })

  const update = useMutation(updateCreditUnit, {
    onSuccess: async () => {
      await invalidateCreditUnits(queryClient)
      toast.success('Credit unit updated')
      onClose()
    },
    onError: error => toast.error(rpcErrorMessage(error, 'The credit unit could not be updated')),
  })

  // An empty symbol clears it.
  const onSubmit = (values: Values) =>
    update.mutate({
      id: unit.id,
      name: values.name.trim(),
      pluralName: values.pluralName.trim(),
      symbol: values.symbol.trim(),
    })

  return (
    <Dialog open onOpenChange={open => !open && onClose()}>
      <DialogContent className="sm:max-w-[480px]">
        <DialogHeader>
          <DialogTitle>Edit {unit.pluralName}</DialogTitle>
          <DialogDescription>Issued invoices keep the previous names.</DialogDescription>
        </DialogHeader>

        <Form {...methods}>
          <form onSubmit={methods.handleSubmit(onSubmit)} className="space-y-4">
            <div className="grid grid-cols-2 gap-4 text-sm">
              <div className="space-y-1">
                <div className="font-medium">Code</div>
                <div className="font-mono text-muted-foreground">{unit.code}</div>
              </div>
              <div className="space-y-1">
                <div className="font-medium">Precision</div>
                <div className="text-muted-foreground">{unit.precision}</div>
              </div>
            </div>
            <div className="grid grid-cols-2 gap-4">
              <InputFormField
                name="name"
                label="Name (singular)"
                control={methods.control}
                autoFocus
              />
              <InputFormField name="pluralName" label="Name (plural)" control={methods.control} />
            </div>

            <InputFormField
              name="symbol"
              label="Symbol (optional)"
              placeholder="cr"
              control={methods.control}
            />

            <DialogFooter>
              <Button variant="outline" type="button" onClick={onClose}>
                Cancel
              </Button>
              <Button type="submit" disabled={update.isPending || !methods.formState.isDirty}>
                {update.isPending ? 'Saving...' : 'Save'}
              </Button>
            </DialogFooter>
          </form>
        </Form>
      </DialogContent>
    </Dialog>
  )
}
