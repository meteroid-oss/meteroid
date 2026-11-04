import { Download } from 'lucide-react'
import { useState } from 'react'

import { env } from '@/lib/env'

import { date, invoiceBadge, invoiceCharge, money } from '../format'
import { usePortalToken, useInvoices } from '../hooks'
import {
  CenterState,
  LinkButton,
  Mono,
  Pager,
  PanelCard,
  Pill,
  Spinner,
} from '../primitives'

import { UsageFilterTag } from './UsageFilterTag'

import type { InvoiceSummary } from '@/rpc/portal/customer/v1/models_pb'

const PER_PAGE = 8

const HEAD: React.CSSProperties = {
  fontSize: 11.5,
  fontWeight: 600,
  letterSpacing: '0.04em',
  textTransform: 'uppercase',
  color: 'var(--mtp-text-3)',
}

const CELL: React.CSSProperties = {
  fontSize: 13,
  color: 'var(--mtp-text)',
  display: 'flex',
  alignItems: 'center',
  minWidth: 0,
}

const ELLIPSIS: React.CSSProperties = {
  maxWidth: '100%',
  overflow: 'hidden',
  textOverflow: 'ellipsis',
  whiteSpace: 'nowrap',
}

/* Grid columns: Invoice · Date · Plan · Amount · Status · Actions */
const COLS = 'minmax(120px,1.1fr) 110px minmax(0,1.2fr) 110px 120px 150px'

export const InvoicesScreen = () => {
  const token = usePortalToken()
  const [page, setPage] = useState(1)

  // The backend pagination is 0-based; our visible page starts at 1.
  const query = useInvoices(page - 1, PER_PAGE)

  const invoices = query.data?.invoices ?? []
  const totalPages = query.data?.paginationMeta?.totalPages ?? 0
  const hasNext = totalPages > 0 ? page < totalPages : invoices.length === PER_PAGE
  const hasPrev = page > 1

  const viewInvoice = (id: string) => {
    window.open(`/portal/invoice-payment/${id}?token=${token}`, '_blank')
  }

  const downloadInvoice = (invoice: InvoiceSummary) => {
    if (!invoice.documentSharingKey) return
    window.open(
      `${env.meteroidRestApiUri}/files/v1/invoice/pdf/${invoice.id}?token=${invoice.documentSharingKey}`,
      '_blank'
    )
  }

  const header = (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
      <h1 style={{ fontSize: 22, fontWeight: 600, letterSpacing: '-0.02em', margin: 0 }}>
        Invoices
      </h1>
      <p style={{ fontSize: 13, color: 'var(--mtp-text-2)', margin: 0 }}>
        Your billing history and downloadable documents.
      </p>
    </div>
  )

  if (query.isLoading) {
    return (
      <div style={{ display: 'flex', flexDirection: 'column', gap: 22 }}>
        {header}
        <div
          style={{
            display: 'flex',
            justifyContent: 'center',
            padding: '48px 0',
            color: 'var(--mtp-text-3)',
          }}
        >
          <Spinner size={20} />
        </div>
      </div>
    )
  }

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 22 }}>
      {header}

      {invoices.length === 0 ? (
        <PanelCard title="Invoices">
          <CenterState
            title="No invoices yet"
            hint="Invoices will appear here once your first billing cycle closes."
          />
        </PanelCard>
      ) : (
        <PanelCard title="Invoices">
          {/* Column header */}
          <div
            style={{
              display: 'grid',
              gridTemplateColumns: COLS,
              gap: 12,
              padding: '10px 20px',
              borderBottom: '1px solid var(--mtp-border)',
              background: 'var(--mtp-surface-2)',
            }}
          >
            <span style={HEAD}>Invoice</span>
            <span style={HEAD}>Date</span>
            <span style={HEAD}>Plan</span>
            <span style={{ ...HEAD, justifySelf: 'end' }}>Amount</span>
            <span style={HEAD}>Status</span>
            <span style={{ ...HEAD, justifySelf: 'end' }}>Actions</span>
          </div>

          {invoices.map((invoice, i) => {
            const { charged, creditLine } = invoiceCharge(invoice)
            return (
            <div
              key={invoice.id}
              className="mtp-hoverable"
              style={{
                display: 'grid',
                gridTemplateColumns: COLS,
                gap: 12,
                padding: '13px 20px',
                alignItems: 'center',
                borderTop: i === 0 ? 'none' : '1px solid var(--mtp-border)',
              }}
            >
              <div style={{ ...CELL, fontWeight: 500 }}>
                <Mono
                  style={{
                    overflow: 'hidden',
                    textOverflow: 'ellipsis',
                    whiteSpace: 'nowrap',
                  }}
                >
                  {invoice.invoiceNumber || '—'}
                </Mono>
              </div>
              <div style={{ ...CELL, color: 'var(--mtp-text-2)' }}>
                {date(invoice.invoiceDate) || '—'}
              </div>
              <div
                style={{
                  ...CELL,
                  color: 'var(--mtp-text-2)',
                  flexDirection: 'column',
                  alignItems: 'flex-start',
                  gap: 2,
                }}
              >
                <span style={{ display: 'flex', alignItems: 'baseline', gap: 8, minWidth: 0 }}>
                  <span style={ELLIPSIS}>{invoice.planName || '—'}</span>
                  <UsageFilterTag subscriptionId={invoice.subscriptionId} />
                </span>
                {creditLine && (
                  <span style={{ ...ELLIPSIS, fontSize: 12, color: 'var(--mtp-text-3)' }}>
                    {creditLine}
                  </span>
                )}
              </div>
              <div style={{ ...CELL, justifyContent: 'flex-end' }}>
                <Mono style={{ fontWeight: 500 }}>{money(charged, invoice.currency)}</Mono>
              </div>
              <div style={CELL}>
                <Pill badge={invoiceBadge(invoice)} />
              </div>
              <div
                style={{
                  ...CELL,
                  justifyContent: 'flex-end',
                  gap: 14,
                }}
              >
                <LinkButton onClick={() => viewInvoice(invoice.id)}>View</LinkButton>
                {invoice.documentSharingKey && (
                  <LinkButton
                    onClick={() => downloadInvoice(invoice)}
                    style={{ color: 'var(--mtp-text-2)' }}
                  >
                    <Download size={13} />
                    Download
                  </LinkButton>
                )}
              </div>
            </div>
            )
          })}

          {(hasPrev || hasNext) && (
            <Pager
              page={page}
              totalPages={totalPages}
              hasPrev={hasPrev}
              hasNext={hasNext}
              onPrev={() => setPage(p => Math.max(1, p - 1))}
              onNext={() => setPage(p => p + 1)}
            />
          )}
        </PanelCard>
      )}
    </div>
  )
}
