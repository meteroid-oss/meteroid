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
} from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { toast } from 'sonner'
import { z } from 'zod'

import { formatWalletAmount, walletTitle } from '@/features/credit/format'
import { invalidateWallets } from '@/features/credit/labels'
import {
  WalletUnitPriceSection,
  unitPriceIssue,
  useWalletUnitPricing,
} from '@/features/credit/walletUnitPrice'
import { useZodForm } from '@/hooks/useZodForm'
import { configureWallet } from '@/rpc/api/wallets/v1/wallets-WalletsService_connectquery'

import type { WalletSettingsDialogContext } from '@/edition/types'

// Blank sells at the list price.
const schema = z
  .object({ unitPrice: z.string() })
  .superRefine((values, ctx) => {
    const issue = unitPriceIssue(values.unitPrice)
    if (issue) ctx.addIssue({ code: 'custom', path: ['unitPrice'], message: issue })
  })

/** The agreed price per credit of a credit unit wallet, its only setting edited here. */
export const WalletPriceDialog = ({ wallet, customerCurrency, onClose }: WalletSettingsDialogContext) => {
  const queryClient = useQueryClient()
  const { listPrice, agreedPrice, staleAgreedPrice } = useWalletUnitPricing(
    wallet,
    customerCurrency
  )
  const methods = useZodForm({ schema, defaultValues: { unitPrice: agreedPrice ?? '' } })
  const mutation = useMutation(configureWallet, {
    onSuccess: () => invalidateWallets(queryClient),
  })

  // The request replaces the thresholds and the access rule, so the stored ones are sent back.
  const onSubmit = async ({ unitPrice }: z.infer<typeof schema>) => {
    try {
      await mutation.mutateAsync({
        customerId: wallet.customerId,
        walletId: wallet.id,
        lowBalanceThresholds: wallet.lowBalanceThresholds,
        usageAtZero: wallet.usageAtZero,
        unitPrice: wallet.creditUnit && unitPrice.trim() ? unitPrice.trim() : undefined,
      })
    } catch (error) {
      toast.error(ConnectError.from(error).rawMessage || 'The wallet settings could not be saved')
      return
    }
    toast.success('Wallet settings saved')
    onClose()
  }

  const description = `Replaces the list price for purchases and overage, before tax.${
    wallet.autoTopUp?.enabled
      ? ' A price above the one the automatic top-up was authorized at stops it until the customer confirms it.'
      : ''
  }`

  return (
    <Dialog open onOpenChange={open => !open && onClose()}>
      <DialogContent className="max-h-[90vh] overflow-y-auto">
        <DialogHeader>
          <DialogTitle>{walletTitle(wallet)} settings</DialogTitle>
          <DialogDescription>
            {`Credit on account: ${formatWalletAmount(wallet.available, wallet)}`}
          </DialogDescription>
        </DialogHeader>
        <Form {...methods}>
          <form onSubmit={methods.handleSubmit(onSubmit)}>
            <WalletUnitPriceSection
              control={methods.control}
              customerCurrency={customerCurrency}
              listPrice={listPrice}
              staleAgreedPrice={staleAgreedPrice}
              description={description}
              divided={false}
            />
            <DialogFooter className="mt-6">
              <Button type="button" variant="outline" onClick={onClose}>
                Cancel
              </Button>
              <Button type="submit" disabled={mutation.isPending}>
                {mutation.isPending ? 'Saving…' : 'Save'}
              </Button>
            </DialogFooter>
          </form>
        </Form>
      </DialogContent>
    </Dialog>
  )
}
