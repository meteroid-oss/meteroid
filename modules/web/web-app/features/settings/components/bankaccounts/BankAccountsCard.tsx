import { createConnectQueryKey , useMutation } from '@connectrpc/connect-query'
import { Badge, Button, Card, Spinner } from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { Plus, Trash2 } from 'lucide-react'
import { useState } from 'react'
import { toast } from 'sonner'

import ConfirmationModal from '@/components/ConfirmationModal'
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
        queryKey: createConnectQueryKey(listBankAccounts),
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
      <Card className="px-8 py-6 max-w-[950px]">
        <div className="space-y-4">
          <div className="flex items-center justify-between">
            <div>
              <h3 className="font-medium text-lg">Bank Accounts</h3>
              <p className="text-sm text-muted-foreground mt-1">
                Manage bank accounts for receiving payments via bank transfer
              </p>
            </div>
            <Button size="sm" onClick={() => setShowAddModal(true)} className="gap-2">
              <Plus className="h-4 w-4" />
              Add Bank Account
            </Button>
          </div>

          <div className="border rounded-lg">
            {bankAccountsQuery.isLoading ? (
              <div className="p-8 text-center">
                <Spinner />
              </div>
            ) : bankAccountsQuery.data?.accounts.length === 0 ? (
              <div className="p-8 text-center text-muted-foreground text-xs">
                No bank accounts configured. Add one to enable bank transfer payments.
              </div>
            ) : (
              <div className="divide-y">
                {bankAccountsQuery.data?.accounts.map(account => (
                  <div key={account.id} className="p-4 flex items-center justify-between">
                    <div className="space-y-1">
                      <div className="flex items-center gap-2">
                        <span className="font-medium">
                          {account.data?.bankName || 'Unknown Bank'}
                        </span>
                        <Badge variant="outline" size="sm">
                          {account.data?.currency}
                        </Badge>
                        <span className="text-sm text-muted-foreground">
                          {account.data?.country}
                        </span>
                      </div>
                      <p className="text-sm text-muted-foreground font-mono">
                        {formatBankAccount(account)}
                      </p>
                    </div>
                    <Button
                      size="sm"
                      variant="ghost"
                      onClick={() => setAccountToDelete(account)}
                      className="text-destructive hover:text-destructive"
                    >
                      <Trash2 className="h-4 w-4" />
                    </Button>
                  </div>
                ))}
              </div>
            )}
          </div>
        </div>
      </Card>

      <AddBankAccountModal open={showAddModal} onClose={() => setShowAddModal(false)} />

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
