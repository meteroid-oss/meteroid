import { useMutation } from '@connectrpc/connect-query'
import { useState } from 'react'
import { toast } from 'sonner'

import { BillingInfo } from '@/features/checkout/components/BillingInfo'
import { AddPaymentMethodDialog } from '@/pages/portal/customer/AddPaymentMethodDialog'
import { setDefaultPaymentMethod } from '@/rpc/portal/shared/v1/shared-PortalSharedService_connectquery'

import { BrandChip } from '../PaymentMethodChip'
import { usePortalNav } from '../context'
import { pmLabel, pmSubLabel } from '../format'
import { LinkButton, PanelCard, PButton, Pill } from '../primitives'

import type { CSSProperties } from 'react'

/* ---------------------------------------------------------------- info rows */

const InfoRow = ({
  label,
  value,
  multiline,
}: {
  label: string
  value: React.ReactNode
  multiline?: boolean
}) => (
  <div
    style={{
      display: 'flex',
      alignItems: multiline ? 'flex-start' : 'center',
      justifyContent: 'space-between',
      gap: 16,
      padding: '12px 20px',
      borderTop: '1px solid var(--mtp-border)',
    }}
  >
    <span style={{ fontSize: 13, color: 'var(--mtp-text-2)', flex: '0 0 auto' }}>{label}</span>
    <span
      style={{
        fontSize: 13,
        fontWeight: 500,
        color: 'var(--mtp-text)',
        textAlign: 'right',
        lineHeight: 1.5,
      }}
    >
      {value}
    </span>
  </div>
)

const initials = (email: string) => {
  const handle = email.split('@')[0] ?? ''
  const parts = handle.split(/[._-]/).filter(Boolean)
  const chars = parts.length >= 2 ? parts[0][0] + parts[1][0] : handle.slice(0, 2)
  return chars.toUpperCase() || '?'
}

const avatarStyle: CSSProperties = {
  width: 30,
  height: 30,
  borderRadius: '50%',
  flex: '0 0 30px',
  display: 'flex',
  alignItems: 'center',
  justifyContent: 'center',
  background: 'var(--mtp-accent-weak)',
  color: 'var(--mtp-accent-ink)',
  fontSize: 11.5,
  fontWeight: 600,
}

/* ----------------------------------------------------------------- screen */

export const SettingsScreen = () => {
  const { overview, refetchOverview } = usePortalNav()
  const customer = overview.customer
  const methods = overview.paymentMethods

  const [addOpen, setAddOpen] = useState(false)
  const [editing, setEditing] = useState(false)

  const setDefaultMut = useMutation(setDefaultPaymentMethod)
  const [defaultingId, setDefaultingId] = useState<string | null>(null)
  const handleSetDefault = async (id: string) => {
    setDefaultingId(id)
    try {
      await setDefaultMut.mutateAsync({ paymentMethodId: id })
      refetchOverview()
    } catch (e) {
      toast.error(e instanceof Error ? e.message : 'Unable to set default')
    } finally {
      setDefaultingId(null)
    }
  }

  const handleEditingChange = (next: boolean) => {
    setEditing(next)
    if (!next) refetchOverview()
  }

  // Build the contacts list: primary billing email + CC invoicing emails.
  const contacts: { email: string; role: 'Primary' | 'CC' }[] = []
  if (customer?.billingEmail) contacts.push({ email: customer.billingEmail, role: 'Primary' })
  for (const email of customer?.invoicingEmails ?? []) {
    if (email && email !== customer?.billingEmail) contacts.push({ email, role: 'CC' })
  }

  const addr = customer?.billingAddress

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 22 }}>
      {/* 1) Payment methods */}
      <PanelCard
        title="Payment methods"
        action={
          (overview.cardConnectionId || overview.directDebitConnectionId) && (
            <PButton variant="secondary" size="sm" onClick={() => setAddOpen(true)}>
              Add payment method
            </PButton>
          )
        }
      >
        {methods.length === 0 ? (
          <div style={{ padding: '20px', fontSize: 13, color: 'var(--mtp-text-2)' }}>
            No payment methods on file.
          </div>
        ) : (
          methods.map((pm, i) => {
            const isDefault = pm.id === customer?.currentPaymentMethodId
            return (
              <div
                key={pm.id}
                style={{
                  display: 'flex',
                  alignItems: 'center',
                  gap: 12,
                  padding: '14px 20px',
                  borderTop: i === 0 ? 'none' : '1px solid var(--mtp-border)',
                }}
              >
                <BrandChip pm={pm} />
                <div style={{ display: 'flex', flexDirection: 'column', gap: 2, minWidth: 0 }}>
                  <span style={{ fontSize: 13.5, fontWeight: 600, color: 'var(--mtp-text)' }}>
                    {pmLabel(pm)}
                  </span>
                  <span style={{ fontSize: 12, color: 'var(--mtp-text-3)' }}>{pmSubLabel(pm)}</span>
                </div>
                <span style={{ marginLeft: 'auto' }}>
                  {isDefault ? (
                    <Pill badge={{ label: 'Default', tone: 'ok' }} dot />
                  ) : (
                    <PButton
                      variant="secondary"
                      size="sm"
                      loading={defaultingId === pm.id}
                      onClick={() => handleSetDefault(pm.id)}
                    >
                      Set default
                    </PButton>
                  )}
                </span>
              </div>
            )
          })
        )}
      </PanelCard>

      {/* 2) Billing details */}
      <PanelCard
        title="Billing details"
        action={!editing && <LinkButton onClick={() => setEditing(true)}>Edit</LinkButton>}
      >
        {editing && customer ? (
          <div style={{ padding: '18px 20px' }}>
            <BillingInfo customer={customer} isEditing setIsEditing={handleEditingChange} />
          </div>
        ) : (
          <>
            <InfoRow label="Company name" value={customer?.name || '—'} />
            <InfoRow label="Billing email" value={customer?.billingEmail || '—'} />
            <InfoRow label="Tax ID / VAT" value={customer?.vatNumber || '—'} />
            <InfoRow
              label="Billing address"
              multiline
              value={
                addr && (addr.line1 || addr.city || addr.country) ? (
                  <span>
                    {addr.line1 && (
                      <>
                        {addr.line1}
                        <br />
                      </>
                    )}
                    {addr.line2 && (
                      <>
                        {addr.line2}
                        <br />
                      </>
                    )}
                    {(addr.city || addr.zipCode) && (
                      <>
                        {[addr.city, addr.zipCode].filter(Boolean).join(' ')}
                        <br />
                      </>
                    )}
                    {addr.country}
                  </span>
                ) : (
                  '—'
                )
              }
            />
            <InfoRow label="Currency" value={customer?.currency || '—'} />
          </>
        )}
      </PanelCard>

      {/* 3) Billing contacts */}
      {contacts.length > 0 && (
        <PanelCard title="Billing contacts">
          {contacts.map((c, i) => (
            <div
              key={c.email}
              style={{
                display: 'flex',
                alignItems: 'center',
                gap: 12,
                padding: '12px 20px',
                borderTop: i === 0 ? 'none' : '1px solid var(--mtp-border)',
              }}
            >
              <div style={avatarStyle}>{initials(c.email)}</div>
              <span style={{ fontSize: 13, fontWeight: 500, color: 'var(--mtp-text)' }}>
                {c.email}
              </span>
              <span style={{ marginLeft: 'auto', fontSize: 11.5, color: 'var(--mtp-text-3)' }}>
                {c.role}
              </span>
            </div>
          ))}
        </PanelCard>
      )}

      {/* Add payment method dialog (reused @md/ui dialog, renders in light mode) */}
      <AddPaymentMethodDialog
        open={addOpen}
        onOpenChange={setAddOpen}
        cardConnectionId={overview.cardConnectionId}
        directDebitConnectionId={overview.directDebitConnectionId}
        onSuccess={() => refetchOverview()}
      />
    </div>
  )
}
