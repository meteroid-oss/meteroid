import { ComponentProps, useState } from 'react'

import { Property } from '@/components/Property'
import { PageSection } from '@/components/layouts/shared/PageSection'
import { CardAction } from '@/features/customers/cards/CardAction'
import { EditBalanceModal } from '@/features/customers/cards/balance/EditBalanceModal'
import { Customer } from '@/rpc/api/customers/v1/models_pb'

type Props = Pick<ComponentProps<typeof PageSection>, 'className'> & {
  customer: Customer
}
export const BalanceCard = ({ customer, className }: Props) => {
  const [editModalVisible, setEditModalVisible] = useState<boolean>(false)

  return (
    <PageSection
      className={className}
      header={{
        title: 'Balance',
        actions: <CardAction onClick={() => setEditModalVisible(true)} />,
      }}
    >
      <div className="flex text-sm">
        <div className="basis-2/4 flex flex-col gap-2">
          <Property label="Balance" value={Number(customer.balanceValueCents)} />
        </div>
      </div>

      <EditBalanceModal
        customer={customer}
        visible={editModalVisible}
        onCancel={() => setEditModalVisible(false)}
      />
    </PageSection>
  )
}
