import { PageSection } from '@/components/layouts/shared/PageSection'
import { Property } from '@/components/molecules/Property'
import { DetailedInvoice } from '@/rpc/api/invoices/v1/models_pb'

interface Props {
  invoice: DetailedInvoice
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
