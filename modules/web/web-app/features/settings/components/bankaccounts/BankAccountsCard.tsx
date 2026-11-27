import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query'
import { Badge, Button } from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { Landmark, Plus, Trash2 } from 'lucide-react'
import { useState } from 'react'
import { toast } from 'sonner'

import ConfirmationModal from '@/components/ConfirmationModal'
import { CopyToClipboardButton } from '@/components/CopyToClipboard'
import { ListSkeleton } from '@/components/Loading'
import {
  SettingsEmptyState,
  SettingsRow,
  SettingsRowGroup,
  SettingsRowIcon,
  SettingsSection,
} from '@/features/settings/components/SettingsSection'
import { useQuery } from '@/lib/connectrpc'
import {
  deleteBankAccount,
  listBankAccounts,
} from '@/rpc/api/bankaccounts/v1/bankaccounts-BankAccountsService_connectquery'
import { BankAccount } from '@/rpc/api/bankaccounts/v1/models_pb'

import { AddBankAccountModal } from './AddBankAccountModal'

export const BankAccountsCard = () => {
  const queryClient = useQueryClient()
  const [showAddModal, setShowAddModal] = useState(false)
  const [accountToDelete, setAccountToDelete] = useState<BankAccount | null>(null)

  const bankAccountsQuery = useQuery(listBankAccounts)

  const deleteBankAccountMut = useMutation(deleteBankAccount, {
    onSuccess: () => {
      queryClient.invalidateQueries({
        queryKey: createConnectQueryKey({
          schema: listBankAccounts,
          cardinality: 'finite'
        }),
      })
      toast.success('Bank account deleted')
      setAccountToDelete(null)
    },
    onError: () => {
      toast.error('Failed to delete bank account')
    },
  })

  const formatBankAccount = (account: BankAccount) => {
    const data = account.data
    if (!data) return 'No account data'

    switch (data.format.case) {
      case 'ibanBicSwift':
        return `IBAN: ${data.format.value.iban}`
      case 'accountNumberBicSwift':
        return `Account: ${data.format.value.accountNumber}`
      case 'accountNumberRoutingNumber':
        return `Account: ${data.format.value.accountNumber} / Routing: ${data.format.value.routingNumber}`
      case 'sortCodeAccountNumber':
        return `Sort Code: ${data.format.value.sortCode} / Account: ${data.format.value.accountNumber}`
      default:
        return 'Unknown format'
    }
  }

  return (
    <>
      {/* A section like the routing above it: its title, and one row per account. */}
      <SettingsSection
        title="Bank accounts"
        description="Manage bank accounts for receiving payments via bank transfer."
        action={
          <Button size="sm" variant="secondary" hasIcon onClick={() => setShowAddModal(true)}>
            <Plus size={14} />
            Add bank account
          </Button>
        }
      >
        <SettingsRowGroup>
          {bankAccountsQuery.isLoading ? (
            <ListSkeleton className="p-4" />
          ) : bankAccountsQuery.data?.accounts.length === 0 ? (
            <SettingsEmptyState framed={false}>
              No bank accounts configured. Add one to enable bank transfer payments.
            </SettingsEmptyState>
          ) : (
            bankAccountsQuery.data?.accounts.map(account => (
              <SettingsRow
                key={account.id}
                title={
                  <>
                    <SettingsRowIcon>
                      <Landmark />
                    </SettingsRowIcon>
                    {account.data?.bankName || 'Unknown Bank'}
                    <Badge variant="outline" size="sm">
                      {account.data?.currency}
                    </Badge>
                    <span className="font-normal text-muted-foreground">
                      {account.data?.country}
                    </span>
                  </>
                }
                description={<span className="font-mono">{formatBankAccount(account)}</span>}
              >
                <span className="text-xs text-muted-foreground font-mono">ID:</span>
                <CopyToClipboardButton
                  text={account.id}
                  buttonClassName="h-5 text-xs text-muted-foreground font-mono"
                />
                <Button
                  size="sm"
                  variant="ghost"
                  onClick={() => setAccountToDelete(account)}
                  className="text-destructive hover:text-destructive"
                  aria-label="Delete bank account"
                >
                  <Trash2 className="h-4 w-4" />
                </Button>
              </SettingsRow>
            ))
          )}
        </SettingsRowGroup>
      </SettingsSection>

      <AddBankAccountModal open={showAddModal} onClose={() => setShowAddModal(false)}/>

      <ConfirmationModal
        visible={!!accountToDelete}
        onSelectConfirm={() =>
          accountToDelete && deleteBankAccountMut.mutate({ id: accountToDelete.id })
        }
        onSelectCancel={() => setAccountToDelete(null)}
        header="Delete Bank Account"
        danger
        buttonLabel="Delete"
      />
    </>
  )
}
