import { PageSection } from '@/components/layouts/shared/PageSection'
import { Property } from '@/components/molecules/Property'
import { Customer } from '@/rpc/api/customers/v1/models_pb'

interface Props {
  invoice: Customer
}

export const CustomerCard = ({ invoice }: Props) => {
  return (
    <PageSection
      header={{
        title: 'Customer',
      }}
    >
      <Property label="Id" value={invoice.customerId} />
      <Property label="Name" value={invoice.customerName} />
    </PageSection>
  )
}
