import { PageSection } from '@/components/layouts/shared/PageSection'
import { Property } from '@/components/molecules/Property'
import { Customer } from '@/rpc/api/customers/v1/models_pb'

interface Props {
  customer: Customer
}

export const CustomerCard = ({ customer }: Props) => {
  return (
    <PageSection
      header={{
        title: 'Customer',
      }}
    >
      <div className="grid grid-cols-2">
        <Property label="Name" value={customer.name} />
        <Property label="Alias" value={customer.alias} />
        <Property label="Email" value={customer.email} />
      </div>
    </PageSection>
  )
}
