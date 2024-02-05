import { ButtonAlt } from '@ui/components'
import { useState } from 'react'

import { PageSection } from '@/components/layouts/shared/PageSection'
import { EditAddressModal } from '@/features/customers/cards/address/EditAddressModal'
import { Address, Customer } from '@/rpc/api/customers/v1/models_pb'

interface Props {
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

export const AddressCard = ({ customer }: Props) => {
  const [editModalVisible, setEditModalVisible] = useState<boolean>(false)

  return (
    <PageSection
      header={{
        title: 'Addresses',
        actions: (
          <ButtonAlt type="outline" onClick={() => setEditModalVisible(true)} className="py-1.5 ">
            Edit
          </ButtonAlt>
        ),
      }}
    >
      <div className="flex text-sm">
        <div className="basis-2/4 flex flex-col gap-2">
          <span className="text-slate-1000">Billing address</span>
          {customer.billingAddress && <AddressLines address={customer.billingAddress} />}
        </div>
        <div className="basis-2/4 flex flex-col gap-2">
          <span className="text-slate-1000">Shipping address</span>
          {customer.shippingAddress?.sameAsBilling ? (
            <span className="text-slate-1000 italic">Same as billing address</span>
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
