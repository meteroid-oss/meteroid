import { usePortalNav } from '../context'
import { date, invoiceBadge, invoiceCharge, money } from '../format'
import { LinkButton, Mono, PanelCard, Pill } from '../primitives'

import { UsageFilterTag } from './UsageFilterTag'

import type { InvoiceSummary } from '@/rpc/portal/customer/v1/models_pb'

export const RecentInvoices = ({ invoices }: { invoices: InvoiceSummary[] }) => {
  const { goTo } = usePortalNav()
  if (invoices.length === 0) return null
  return (
    <PanelCard
      title="Recent invoices"
      action={<LinkButton onClick={() => goTo('invoices')}>View all</LinkButton>}
    >
      <ul style={{ listStyle: 'none', margin: 0, padding: 0 }}>
        {invoices.slice(0, 3).map((invoice, i) => {
          const { charged, creditLine } = invoiceCharge(invoice)
          const sub = creditLine ?? invoice.planName
          return (
            <li
              key={invoice.id}
              style={{
                display: 'flex',
                flexWrap: 'wrap',
                justifyContent: 'space-between',
                alignItems: 'center',
                gap: 8,
                padding: '14px 20px',
                borderTop: i === 0 ? 'none' : '1px solid var(--mtp-border)',
              }}
            >
              <div style={{ display: 'flex', flexDirection: 'column', gap: 2, minWidth: 0 }}>
                <span style={{ fontSize: 13.5, fontWeight: 500 }}>
                  {[date(invoice.invoiceDate), invoice.invoiceNumber].filter(Boolean).join(' · ')}
                </span>
                {sub && (
                  <span
                    style={{
                      display: 'inline-flex',
                      alignItems: 'baseline',
                      gap: 8,
                      fontSize: 12.5,
                      color: 'var(--mtp-text-2)',
                    }}
                  >
                    {sub}
                    {!creditLine && <UsageFilterTag subscriptionId={invoice.subscriptionId} />}
                  </span>
                )}
              </div>
              <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
                <Mono style={{ fontWeight: 600 }}>{money(charged, invoice.currency)}</Mono>
                <Pill badge={invoiceBadge(invoice)} />
              </div>
            </li>
          )
        })}
      </ul>
    </PanelCard>
  )
}
