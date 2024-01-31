import { LinkIcon } from 'lucide-react'
import { Link } from 'react-router-dom'

import { PageSection } from '@/components/layouts/shared/PageSection'
import { Property } from '@/components/molecules/Property'
import { useTenant } from '@/hooks/useTenant'
import { DetailedInvoice } from '@/rpc/api/invoices/v1/models_pb'

interface Props {
  invoice: DetailedInvoice
}

export const SubscriptionCard = ({ invoice }: Props) => {
  const { tenant } = useTenant()

  return (
    <PageSection
      header={{
        title: 'Subscription',
      }}
    >
      <Property label="Id" value={invoice.subscriptionId} />
      <Property
        label="Plan"
        value={
          <div className="flex flex-row items-center gap-2">
            {invoice.planName}
            <Link
              className="text-slate-1000 hover:text-slate-1200"
              to={`/tenant/${tenant?.slug}/billing/default/plans/${invoice.planExternalId}`}
            >
              <LinkIcon size="1em" />
            </Link>
            <span className="text-xs text-slate-1000">(version: {invoice.planVersion})</span>
          </div>
        }
      />
    </PageSection>
  )
}
