import { PageSection } from '@/components/layouts/shared/PageSection'
import { Customer } from '@/rpc/api/customers/v1/models_pb'

interface Props {
  customer: Customer
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
          <div className="flex flex-col gap-0.5">
            <span>{customer.billingAddressLine1}</span>
            <span>{customer.billingAddressLine2}</span>
            <span>{customer.billingAddressCity}</span>
            <span>{customer.billingAddressState}</span>
            <span>{customer.billingAddressCountry}</span>
            <span>{customer.billingAddressZipcode}</span>
          </div>
        </div>
        <div className="basis-2/4 flex flex-col gap-2">
          <span className="text-slate-1000">Shipping address</span>
          {customer.shippingAddressSame ? (
            <span className="text-slate-1000 italic">Same as billing address</span>
          ) : (
            <div className="flex flex-col gap-0.5">
              <span>{customer.shippingAddressLine1}</span>
              <span>{customer.shippingAddressLine2}</span>
              <span>{customer.shippingAddressCity}</span>
              <span>{customer.shippingAddressState}</span>
              <span>{customer.shippingAddressCountry}</span>
              <span>{customer.shippingAddressZipcode}</span>
            </div>
          )}
        </div>
      </div>
    </PageSection>
  )
}
