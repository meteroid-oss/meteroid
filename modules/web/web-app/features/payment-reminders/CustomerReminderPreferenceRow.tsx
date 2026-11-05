import { Flex } from '@md/ui'

import { useQuery } from '@/lib/connectrpc'
import { getCustomerReminderPreference } from '@/rpc/api/paymentreminders/v1/paymentreminders-PaymentRemindersService_connectquery'

import {
  CustomerReminderOverridePopover,
  reminderPreferenceLabel,
} from './CustomerReminderOverridePopover'

export const CustomerReminderPreferenceRow = ({ customerId }: { customerId: string }) => {
  const query = useQuery(
    getCustomerReminderPreference,
    { customerId },
    { enabled: Boolean(customerId), retry: false }
  )
  const preference = query.data?.preference

  return (
    <Flex align="center" justify="between">
      <div className="text-[13px] text-muted-foreground">Payment reminders</div>
      <div className="max-w-[250px] break-words text-right">
        <div className="text-[13px]">
          {preference ? (
            <span className="flex items-center gap-1 justify-end">
              <span>{reminderPreferenceLabel(preference)}</span>
              <CustomerReminderOverridePopover customerId={customerId} preference={preference} />
            </span>
          ) : query.isError ? (
            <span className="text-muted-foreground">
              {query.error.rawMessage || 'Couldn’t load'}
            </span>
          ) : (
            '—'
          )}
        </div>
      </div>
    </Flex>
  )
}
