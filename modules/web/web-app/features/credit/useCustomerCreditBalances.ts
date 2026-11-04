import { formatWalletAmount, walletTitle } from '@/features/credit/format'
import { useQuery } from '@/lib/connectrpc'
import { listCustomerWallets } from '@/rpc/api/wallets/v1/wallets-WalletsService_connectquery'

/** The balance of each wallet; wallets apply at different moments, so never summed. */
export const useCustomerCreditBalances = (customerId: string, enabled: boolean) => {
  const query = useQuery(listCustomerWallets, { customerId }, { enabled })
  return (query.data?.wallets ?? [])
    .filter(wallet => wallet.available !== 0n || wallet.pending !== 0n)
    .map(wallet => ({
      id: wallet.id,
      label: walletTitle(wallet),
      formatted: formatWalletAmount(wallet.live?.availableAfterUsage ?? wallet.available, wallet),
    }))
}
