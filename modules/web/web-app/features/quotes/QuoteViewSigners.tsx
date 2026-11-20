import { cn } from '@md/ui'
import { Check, PenLine } from 'lucide-react'

import { sameEmail, signerRoleLabel } from '@/features/quotes/detail/signers'
import { Quote, QuoteSignature, QuoteSignerRole } from '@/rpc/api/quotes/v1/models_pb'
import { parseAndFormatDateTime } from '@/utils/date'

export interface QuoteSignerSlot {
  key: string
  role: QuoteSignerRole
  name: string
  signed: boolean
  signedAt?: string
  /** The viewer: their unsigned block is the call to sign. */
  current?: boolean
}

/** The quote's signers (recipients, then countersigners) with their signatures. */
export const signerSlotsFromQuote = (
  quote: Quote | undefined,
  signatures: QuoteSignature[]
): QuoteSignerSlot[] => {
  const used = new Set<string>()
  const find = (email: string, role: QuoteSignerRole) => {
    const signature = signatures.find(
      s => !used.has(s.id) && s.signerRole === role && sameEmail(s.signedByEmail, email)
    )
    if (signature) used.add(signature.id)
    return signature
  }

  const slots: QuoteSignerSlot[] = []
  for (const recipient of quote?.recipients ?? []) {
    const signature = find(recipient.email, QuoteSignerRole.SIGNER_CUSTOMER)
    slots.push({
      key: `customer:${recipient.email}`,
      role: QuoteSignerRole.SIGNER_CUSTOMER,
      name: recipient.name || recipient.email,
      signed: Boolean(signature),
      signedAt: signature?.signedAt,
    })
  }
  for (const countersigner of quote?.countersigners ?? []) {
    const signature = find(countersigner.email, QuoteSignerRole.SIGNER_COUNTERSIGNER)
    slots.push({
      key: `countersigner:${countersigner.email}`,
      role: QuoteSignerRole.SIGNER_COUNTERSIGNER,
      name: countersigner.name,
      signed: Boolean(signature),
      signedAt: signature?.signedAt,
    })
  }
  // Signatures of people no longer listed (a recipient removed after signing) stay on the record.
  for (const signature of signatures.filter(s => !used.has(s.id))) {
    slots.push({
      key: `signature:${signature.id}`,
      role: signature.signerRole,
      name: signature.signedByName,
      signed: true,
      signedAt: signature.signedAt,
    })
  }
  return slots
}

const signerRoleLabelFor = (slot: QuoteSignerSlot) =>
  slot.current ? 'Your signature' : signerRoleLabel(slot.role)

interface Props {
  signers: QuoteSignerSlot[]
  onSign?: () => void
  signDisabled?: boolean
}

export const QuoteSignatureBlocks = ({ signers, onSign, signDisabled }: Props) => {
  if (signers.length === 0) {
    return (
      <div className="rounded-lg border-2 border-dashed border-border p-8 text-center text-sm text-muted-foreground">
        No signers
      </div>
    )
  }

  return (
    <div className="grid grid-cols-1 gap-4 sm:grid-cols-2">
      {signers.map(slot => {
        const heading = (
          <>
            <span
              className={cn(
                'text-xs',
                slot.current && !slot.signed
                  ? 'font-medium text-foreground'
                  : 'text-muted-foreground'
              )}
            >
              {signerRoleLabelFor(slot)}
            </span>
            <span className="font-semibold text-foreground">{slot.name}</span>
          </>
        )

        if (slot.current && !slot.signed && onSign) {
          return (
            <button
              key={slot.key}
              type="button"
              onClick={onSign}
              disabled={signDisabled}
              className="flex flex-col items-start gap-1 rounded-lg border-2 border-dashed border-brand bg-brand/10 p-4 text-left transition-colors hover:bg-brand/15 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-60"
            >
              {heading}
              <span className="mt-1 inline-flex items-center gap-1.5 text-sm font-medium text-foreground">
                <PenLine aria-hidden className="h-4 w-4" />
                Click to sign
              </span>
            </button>
          )
        }

        return (
          <div
            key={slot.key}
            className={cn(
              'flex flex-col gap-1 rounded-lg p-4',
              slot.signed
                ? 'border border-border bg-muted/40'
                : 'border border-dashed border-border'
            )}
          >
            {heading}
            {slot.signed ? (
              <span className="flex items-center gap-1.5 text-xs text-success">
                <Check aria-hidden className="h-3.5 w-3.5" />
                {slot.signedAt ? `Signed ${parseAndFormatDateTime(slot.signedAt)}` : 'Signed'}
              </span>
            ) : (
              <span className="text-xs text-muted-foreground">Awaiting signature</span>
            )}
          </div>
        )
      })}
    </div>
  )
}
