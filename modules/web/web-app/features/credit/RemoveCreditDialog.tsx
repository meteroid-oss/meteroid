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
} from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { useState } from 'react'
import { toast } from 'sonner'
import { z } from 'zod'

import { optionalAmountSchema, optionalMinorUnits } from '@/features/credit/amounts'
import { formatWalletAmount } from '@/features/credit/format'
import { invalidateWallets } from '@/features/credit/labels'
import { useZodForm } from '@/hooks/useZodForm'
import { removeCredit } from '@/rpc/api/wallets/v1/wallets-WalletsService_connectquery'

import type { CreditGrant, Wallet } from '@/rpc/api/wallets/v1/models_pb'

type Props = {
  open: boolean
  onOpenChange: (open: boolean) => void
  wallet: Wallet
  grant: CreditGrant
}

const buildSchema = (currency: string, remaining: bigint) =>
  z.object({
    amount: optionalAmountSchema(currency)
      .refine(value => optionalMinorUnits(value, currency) !== 0n, 'Enter an amount above zero')
      .refine(
        value => (optionalMinorUnits(value, currency) ?? remaining) <= remaining,
        'This grant does not have that much left'
      ),
    reason: z.string().trim().min(1, 'Say why the credit is removed').max(1024),
  })

// Only credit added by hand is removed here; purchased credit is corrected with a credit note.
export const RemoveCreditDialog = ({ open, onOpenChange, wallet, grant }: Props) => {
  const queryClient = useQueryClient()
  const [idempotencyKey] = useState(() => crypto.randomUUID())
  const schema = buildSchema(wallet.currency, grant.remaining)
  const methods = useZodForm({ schema, defaultValues: { amount: '', reason: '' } })
  const mutation = useMutation(removeCredit, {
    onSuccess: () => invalidateWallets(queryClient),
  })

  const onSubmit = async (values: z.infer<typeof schema>) => {
    try {
      const response = await mutation.mutateAsync({
        customerId: wallet.customerId,
        creditGrantId: grant.id,
        amount: optionalMinorUnits(values.amount, wallet.currency),
        reason: values.reason,
        idempotencyKey,
      })
      toast.success(`${formatWalletAmount(response.removed, wallet)} removed`)
      onOpenChange(false)
    } catch (error) {
      toast.error(ConnectError.from(error).rawMessage || 'The credit could not be removed')
    }
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Remove credit</DialogTitle>
          <DialogDescription>
            {formatWalletAmount(grant.remaining, wallet)} of this grant is left.
          </DialogDescription>
        </DialogHeader>
        <Form {...methods}>
          <form onSubmit={methods.handleSubmit(onSubmit)} className="space-y-4">
            <InputFormField
              control={methods.control}
              name="amount"
              label={`Amount (${wallet.creditUnit?.pluralName ?? wallet.currency})`}
              inputMode="decimal"
              placeholder="Everything left"
              autoFocus
            />
            <InputFormField
              control={methods.control}
              name="reason"
              label="Reason"
              maxLength={1024}
            />
            <DialogFooter>
              <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
                Cancel
              </Button>
              <Button type="submit" variant="destructive" disabled={mutation.isPending}>
                {mutation.isPending ? 'Removing…' : 'Remove credit'}
              </Button>
            </DialogFooter>
          </form>
        </Form>
      </DialogContent>
    </Dialog>
  )
}
