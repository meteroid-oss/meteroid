import { ComponentProps, useState } from 'react'

import { PageSection } from '@/components/layouts/shared/PageSection'
import { CardAction } from '@/features/customers/cards/CardAction'
import { EditAddressModal } from '@/features/customers/cards/address/EditAddressModal'
import { Address, Customer } from '@/rpc/api/customers/v1/models_pb'

type Props = Pick<ComponentProps<typeof PageSection>, 'className'> & {
  customer: Customer
}

const AddressLines = ({ address }: { address: Partial<Address> }) => {
  return (
    <div className="flex flex-col gap-0.5">
      <span>{address.line1}</span>
      <span>{address.line2}</span>
      <span>{address.city}</span>
      <span>{address.state}</span>
      <span>{address.country}</span>
      <span>{address.zipCode}</span>
    </div>
  )
}

export const AddressCard = ({ customer, className }: Props) => {
  const [editModalVisible, setEditModalVisible] = useState<boolean>(false)

  return (
    <PageSection
      className={className}
      header={{
        title: 'Addresses',
        actions: <CardAction onClick={() => setEditModalVisible(true)} />,
      }}
    >
      <div className="flex text-sm">
        <div className="basis-2/4 flex flex-col gap-2">
          <span className="text-muted-foreground">Billing address</span>
          {customer.billingAddress && <AddressLines address={customer.billingAddress} />}
        </div>
        <div className="basis-2/4 flex flex-col gap-2">
          <span className="text-muted-foreground">Shipping address</span>
          {customer.shippingAddress?.sameAsBilling ? (
            <span className="text-muted-foreground italic">Same as billing address</span>
          ) : (
            <AddressLines address={customer.shippingAddress?.address || {}} />
          )}
        </div>
      </div>

      <EditAddressModal
        customer={customer}
        visible={editModalVisible}
        onCancel={() => setEditModalVisible(false)}
      />
    </PageSection>
  )
}
