import { createConnectQueryKey, skipToken, useMutation } from '@connectrpc/connect-query'
import { Switch } from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { toast } from 'sonner'

import { SettingsRow, SettingsRowGroup } from '@/features/settings/components/SettingsSection'
import { useInvoicingEntity } from '@/features/settings/hooks/useInvoicingEntity'
import { usePermissions } from '@/hooks/usePermissions'
import { useQuery } from '@/lib/connectrpc'
import {
  getPaymentRemindersEnabled,
  setPaymentRemindersEnabled,
} from '@/rpc/api/customeremails/v1/customeremails-CustomerEmailsService_connectquery'

export const PaymentRemindersSwitch = () => {
  const queryClient = useQueryClient()
  const { selectedEntityId: entityId } = useInvoicingEntity()
  const canManage = usePermissions().has('settings:manage')

  const query = useQuery(
    getPaymentRemindersEnabled,
    entityId ? { invoicingEntityId: entityId } : skipToken
  )
  const mutation = useMutation(setPaymentRemindersEnabled, {
    onSuccess: () =>
      queryClient.invalidateQueries({
        queryKey: createConnectQueryKey({
          schema: getPaymentRemindersEnabled,
          cardinality: undefined,
        }),
      }),
    onError: err => toast.error(err.rawMessage || 'Could not update payment reminders'),
  })

  return (
    <SettingsRowGroup>
      <SettingsRow
        title="Payment reminders"
        description="Remind customers of unpaid invoices 3 days before they are due, on the due date, then 7 and 14 days after."
      >
        <Switch
          checked={query.data?.enabled ?? false}
          disabled={!canManage || !entityId || !query.data || mutation.isPending}
          onCheckedChange={enabled =>
            entityId && mutation.mutate({ invoicingEntityId: entityId, enabled })
          }
          aria-label="Payment reminders"
        />
      </SettingsRow>
    </SettingsRowGroup>
  )
}
