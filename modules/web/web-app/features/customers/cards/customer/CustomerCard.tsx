import dayjs from 'dayjs'
import { ComponentProps, useState } from 'react'

import { Property } from '@/components/Property'
import { PageSection } from '@/components/layouts/shared/PageSection'
import { CardAction } from '@/features/customers/cards/CardAction'
import { EditCustomerModal } from '@/features/customers/cards/customer/EditCustomerModal'
import { Customer } from '@/rpc/api/customers/v1/models_pb'

type Props = Pick<ComponentProps<typeof PageSection>, 'className'> & {
  customer: Customer
}

export const CustomerCard = ({ customer, className }: Props) => {
  const [editModalVisible, setEditModalVisible] = useState<boolean>(false)

  return (
    <PageSection
      className={className}
      header={{
        title: 'Customer',
        actions: <CardAction onClick={() => setEditModalVisible(true)} />,
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
