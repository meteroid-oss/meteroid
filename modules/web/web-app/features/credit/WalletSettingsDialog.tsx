import { slotItems } from '@/edition'
import { WalletPriceDialog } from '@/features/credit/WalletPriceDialog'

import type { WalletSettingsDialogContext } from '@/edition/types'
import type { Wallet } from '@/rpc/api/wallets/v1/models_pb'

const moduleDialog = () => slotItems('wallet.settingsDialog')[0]

/** Whether a wallet has settings to edit: a module's, else a credit unit wallet's agreed price. */
export const hasWalletSettings = (wallet: Pick<Wallet, 'creditUnit'>): boolean =>
  !!moduleDialog() || !!wallet.creditUnit

export const WalletSettingsDialog = (props: WalletSettingsDialogContext) => {
  const Dialog = moduleDialog() ?? WalletPriceDialog
  return <Dialog {...props} />
}
