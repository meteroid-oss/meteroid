import { ButtonAlt } from '@ui/components'
import dayjs from 'dayjs'
import { useState } from 'react'

import { PageSection } from '@/components/layouts/shared/PageSection'
import { Property } from '@/components/molecules/Property'
import { EditCustomerModal } from '@/features/customers/cards/customer/EditCustomerModal'
import { Customer } from '@/rpc/api/customers/v1/models_pb'

interface Props {
  customer: Customer
}

export const CustomerCard = ({ customer }: Props) => {
  const [editModalVisible, setEditModalVisible] = useState<boolean>(false)

  return (
    <PageSection
      header={{
        title: 'Customer',
        actions: (
          <ButtonAlt type="outline" onClick={() => setEditModalVisible(true)} className="py-1.5">
            Edit
          </ButtonAlt>
        ),
      }}
    >
      <div className="flex text-sm">
        <div className="basis-2/4 flex flex-col gap-2">
          <Property label="Name" value={customer.name} />
          <Property label="Alias" value={customer.alias} />
          <Property
            label="Created at"
            value={dayjs(customer.createdAt?.toDate()).format('DD/MM/YY HH:mm')}
          />
        </div>
        <div className="basis-2/4 flex flex-col gap-2">
          <Property label="Email" value={customer.email} />
          <Property label="Invoicing email" value={customer.invoicingEmail} />
          <Property label="Phone" value={customer.phone} />
        </div>
      </div>

      <EditCustomerModal
        customer={customer}
        visible={editModalVisible}
        onCancel={() => setEditModalVisible(false)}
      />
    </PageSection>
  )
}
