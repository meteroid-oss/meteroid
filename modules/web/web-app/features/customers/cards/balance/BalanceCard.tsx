import { ButtonAlt } from '@ui/components'
import { useState } from 'react'

import { PageSection } from '@/components/layouts/shared/PageSection'
import { Property } from '@/components/molecules/Property'
import { EditBalanceModal } from '@/features/customers/cards/balance/EditBalanceModal'
import { Customer } from '@/rpc/api/customers/v1/models_pb'

interface Props {
  customer: Customer
}

export const BalanceCard = ({ customer }: Props) => {
  const [editModalVisible, setEditModalVisible] = useState<boolean>(false)

  return (
    <PageSection
      header={{
        title: 'Balance',
        actions: (
          <ButtonAlt type="outline" onClick={() => setEditModalVisible(true)} className="py-1.5 ">
            Edit
          </ButtonAlt>
        ),
      }}
    >
      <div className="flex text-sm">
        <div className="basis-2/4 flex flex-col gap-2">
          <Property label="Balance" value={customer.balanceValueCents} />
        </div>
        <div className="basis-2/4 flex flex-col gap-2">
          <Property label="Currency" value={customer.balanceCurrency} />
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
