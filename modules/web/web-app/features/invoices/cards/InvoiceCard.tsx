import dayjs from 'dayjs'

import { Property } from '@/components/Property'
import { PageSection } from '@/components/layouts/shared/PageSection'
import { DetailedInvoice, InvoicingProvider } from '@/rpc/api/invoices/v1/models_pb'

interface Props {
  invoice: DetailedInvoice
}

export const InvoiceCard = ({ invoice }: Props) => {
  return (
    <PageSection
      header={{
        title: 'Invoice',
      }}
    >
      <div className="grid grid-cols-2">
        <Property label="Currency" value={invoice.currency} />
        <Property
          label="Created at"
          value={
            invoice.createdAt ? dayjs(invoice.createdAt?.toDate()).format('DD/MM/YY HH:mm') : '-'
          }
        />
        <Property label="Provider" value={InvoicingProvider[invoice.invoicingProvider]} />
        <Property label="Invoice date" value={invoice.invoiceDate} />
        <Property label="Issued" value={String(invoice.issued)} />
        <Property label="Days until due" value={invoice.daysUntilDue} />
        <Property label="Issue attempts" value={invoice.issueAttempts} />
      </div>
    </PageSection>
  )
}
