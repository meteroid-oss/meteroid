import { ConnectError } from '@connectrpc/connect'
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
  SelectFormField,
  SelectItem,
} from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { toast } from 'sonner'
import { z } from 'zod'

import { invalidateWallets } from '@/features/credit/labels'
import { useZodForm } from '@/hooks/useZodForm'
import { useQuery } from '@/lib/connectrpc'
import { listCreditUnits } from '@/rpc/api/creditunits/v1/creditunits-CreditUnitsService_connectquery'
import { createWallet } from '@/rpc/api/wallets/v1/wallets-WalletsService_connectquery'

import type { Wallet } from '@/rpc/api/wallets/v1/models_pb'

type Props = {
  open: boolean
  onOpenChange: (open: boolean) => void
  customerId: string
  currency: string
  /** Adding credit: called with the wallet opened for it (or the customer's existing one). */
  onCreated?: (wallet: Wallet) => void
}

// A wallet holds money in a currency, or credits of one of the tenant's credit units.
const MONEY = '__money__'

const schema = z
  .object({
    holds: z.string(),
    currency: z.string().trim(),
  })
  .superRefine((values, ctx) => {
    if (values.holds === MONEY && !/^[A-Za-z]{3}$/.test(values.currency)) {
      ctx.addIssue({
        code: z.ZodIssueCode.custom,
        path: ['currency'],
        message: 'Enter a three-letter currency code',
      })
    }
  })

export const CreateWalletDialog = ({
  open,
  onOpenChange,
  customerId,
  currency,
  onCreated,
}: Props) => {
  const queryClient = useQueryClient()
  const methods = useZodForm({
    schema,
    defaultValues: { holds: MONEY, currency },
  })
  const holds = methods.watch('holds')
  const units = (useQuery(listCreditUnits, { includeArchived: false }).data?.units ?? []).filter(
    unit => !unit.archived
  )
  const mutation = useMutation(createWallet, {
    onSuccess: () => invalidateWallets(queryClient),
  })

  const onSubmit = async (values: z.infer<typeof schema>) => {
    try {
      const response = await mutation.mutateAsync({
        customerId,
        currency: values.holds === MONEY ? values.currency.toUpperCase() : values.holds,
      })
      if (onCreated && response.wallet) {
        onCreated(response.wallet)
        return
      }
      toast.success('Wallet ready')
      onOpenChange(false)
    } catch (error) {
      toast.error(ConnectError.from(error).rawMessage || 'The wallet could not be created')
    }
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{onCreated ? 'Add credit' : 'New wallet'}</DialogTitle>
          <DialogDescription>
            {onCreated
              ? 'Choose what the credit is in.'
              : "Holds the customer's credit in one currency or credit unit."}
          </DialogDescription>
        </DialogHeader>
        <Form {...methods}>
          <form onSubmit={methods.handleSubmit(onSubmit)} className="space-y-4">
            {units.length > 0 && (
              <SelectFormField control={methods.control} name="holds" label="Holds">
                <SelectItem value={MONEY}>Money</SelectItem>
                {units.map(unit => (
                  <SelectItem key={unit.id} value={unit.code}>
                    {unit.pluralName} ({unit.code})
                  </SelectItem>
                ))}
              </SelectFormField>
            )}
            {holds === MONEY && (
              <InputFormField
                control={methods.control}
                name="currency"
                label="Currency"
                maxLength={3}
              />
            )}
            <DialogFooter>
              <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
                Cancel
              </Button>
              <Button type="submit" disabled={mutation.isPending}>
                {mutation.isPending
                  ? onCreated
                    ? 'Opening…'
                    : 'Creating…'
                  : onCreated
                    ? 'Continue'
                    : 'Create wallet'}
              </Button>
            </DialogFooter>
          </form>
        </Form>
      </DialogContent>
    </Dialog>
  )
}
