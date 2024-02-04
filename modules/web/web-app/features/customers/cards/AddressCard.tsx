import { match, P } from 'ts-pattern'

import { PageSection } from '@/components/layouts/shared/PageSection'
import { Address, Customer } from '@/rpc/api/customers/v1/models_pb'

interface Props {
  customer: Customer
}

const AddressLines = ({ address }: { address: Address }) => {
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
  return (
    <PageSection
      header={{
        title: 'Addresses',
      }}
    >
      <div className="flex text-sm">
        <div className="basis-2/4 flex flex-col gap-2">
          <span className="text-slate-1000">Billing address</span>
          {customer.billingAddress && <AddressLines address={customer.billingAddress} />}
        </div>
        <div className="basis-2/4 flex flex-col gap-2">
          <span className="text-slate-1000">Shipping address</span>
          {match(customer)
            .with({ billingAddress: P.not(P.nullish), shippingAddress: P.nullish }, () => (
              <span className="text-slate-1000 italic">Same as billing address</span>
            ))
            .with({ shippingAddress: P.not(P.nullish) }, c => (
              <AddressLines address={c.shippingAddress} />
            ))
            .otherwise(() => null)}
        </div>
      </div>
    </PageSection>
  )
}
