import { Building } from 'lucide-react'

import { CustomerPaymentMethod_PaymentMethodTypeEnum as PmType } from '@/rpc/api/customers/v1/models_pb'

import type { CustomerPaymentMethod } from '@/rpc/api/customers/v1/models_pb'

/* --------------------------------------------------------------- brand chip */

const CARD_GRADIENT: Record<string, string> = {
  visa: 'linear-gradient(135deg,#1A1F71,#3B5BDB)',
  mastercard: 'linear-gradient(135deg,#EB001B,#F79E1B)',
  amex: 'linear-gradient(135deg,#108168,#1AAE9F)',
  discover: 'linear-gradient(135deg,#E26B0A,#F2A93B)',
}

/** Card-brand gradient / bank icon chip shown next to a payment method. */
export const BrandChip = ({ pm }: { pm: CustomerPaymentMethod }) => {
  const isCard = pm.paymentMethodType === PmType.CARD
  const brand = (pm.cardBrand ?? '').toLowerCase()
  const gradient = CARD_GRADIENT[brand] ?? 'linear-gradient(135deg,#3F3F46,#18181B)'
  return (
    <div
      style={{
        width: 38,
        height: 28,
        borderRadius: 'var(--mtp-r-sm)',
        flex: '0 0 38px',
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
        color: '#fff',
        ...(isCard
          ? { background: gradient }
          : {
              background: 'var(--mtp-surface-2)',
              border: '1px solid var(--mtp-border)',
              color: 'var(--mtp-text-2)',
            }),
      }}
    >
      {isCard ? (
        <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.7">
          <rect x="2" y="5" width="20" height="14" rx="2.5" />
          <path d="M2 10h20" />
        </svg>
      ) : (
        <Building size={16} />
      )}
    </div>
  )
}
