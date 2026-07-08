import { timestampDate } from '@bufbuild/protobuf/wkt'
import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query'
import { Badge, Button, Card } from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { Edit2, RefreshCw } from 'lucide-react'
import { ReactNode } from 'react'
import { toast } from 'sonner'

import { getCountryFlagEmoji, getCountryName } from '@/features/settings/utils'
import {
  getCustomerById,
  refreshVatValidation,
} from '@/rpc/api/customers/v1/customers-CustomersService_connectquery'
import { CustomerSchema, VatNumberValidationStatus } from '@/rpc/api/customers/v1/models_pb'

import type { MessageInitShape } from '@bufbuild/protobuf'
import type { Timestamp } from '@bufbuild/protobuf/wkt'

interface BillingInfoCardProps {
  customer: MessageInitShape<typeof CustomerSchema>
  onEdit: () => void
  title?: string
  actions?: ReactNode // Additional actions next to edit button
  cardVariant?: 'default' | 'accent' | 'accent2'
}

const VAT_BADGES: Partial<
  Record<
    VatNumberValidationStatus,
    { label: string; variant: 'success' | 'destructive' | 'warning' | 'ghost'; title: string }
  >
> = {
  [VatNumberValidationStatus.PENDING]: {
    label: 'Verifying…',
    variant: 'ghost',
    title: 'Verification against VIES is in progress',
  },
  [VatNumberValidationStatus.VALID]: {
    label: 'VIES verified',
    variant: 'success',
    title: 'VAT number is registered in VIES',
  },
  [VatNumberValidationStatus.INVALID]: {
    label: 'Not in VIES',
    variant: 'destructive',
    title: 'VIES has no active registration for this VAT number',
  },
  [VatNumberValidationStatus.UNAVAILABLE]: {
    label: 'Unverified',
    variant: 'warning',
    title: 'VIES was unreachable — verification is retried automatically',
  },
}

export const VatValidationBadge = ({
  customer,
}: {
  customer: BillingInfoCardProps['customer']
}) => {
  const queryClient = useQueryClient()
  const customerId = customer.id

  const refreshMutation = useMutation(refreshVatValidation, {
    onSuccess: () => {
      queryClient.invalidateQueries({
        queryKey: createConnectQueryKey({
          schema: getCustomerById,
          input: { id: customerId },
          cardinality: 'finite',
        }),
      })
    },
    onError: () => toast.error('Failed to start VAT number re-check'),
  })

  const status = customer.vatNumberValidationStatus
  const badge = status ? VAT_BADGES[status] : undefined
  if (!badge) return null

  const checkedAt = customer.vatNumberCheckedAt
    ? timestampDate(customer.vatNumberCheckedAt as Timestamp).toLocaleDateString()
    : undefined

  const title = [
    badge.title,
    customer.vatNumberRegisteredName && `Registered as: ${customer.vatNumberRegisteredName}`,
    customer.vatNumberConsultationNumber &&
      `VIES ref: ${customer.vatNumberConsultationNumber}`,
    checkedAt && `Last checked ${checkedAt}`,
  ]
    .filter(Boolean)
    .join(' — ')

  return (
    <span className="inline-flex items-center gap-1 ml-1.5 align-middle">
      <Badge variant={badge.variant} size="sm" title={title}>
        {badge.label}
      </Badge>
      {status !== VatNumberValidationStatus.PENDING && customerId && (
        <Button
          variant="ghost"
          size="sm"
          className="p-0 h-auto text-muted-foreground"
          title="Re-check against VIES"
          disabled={refreshMutation.isPending}
          onClick={() => refreshMutation.mutate({ customerId })}
        >
          <RefreshCw size={12} />
        </Button>
      )}
    </span>
  )
}

export const BillingInfoCard = ({
  customer,
  onEdit,
  title = 'Billing information',
  actions,
  cardVariant = 'accent',
}: BillingInfoCardProps) => {
  return (
    <>
      <div className="text-sm font-medium">{title}</div>
      <Card className="mb-8 px-6 py-4 mt-2 border-0" variant={cardVariant}>
        <div className="flex justify-between items-start mb-2">
          <div className="text-sm space-y-1">
            <div className="font-medium">{customer.name}</div>
            {customer.billingEmail && (
              <div className="text-muted-foreground">{customer.billingEmail}</div>
            )}
            {customer.billingAddress && (
              <div className="pt-0">
                {customer.billingAddress.line1}
                {customer.billingAddress.line2 && <span>, {customer.billingAddress.line2}</span>}
                {customer.billingAddress.line1 && <br />}
                {customer.billingAddress.city}
                {customer.billingAddress.state && (
                  <span>, {customer.billingAddress.state}</span>
                )}{' '}
                {customer.billingAddress.zipCode}
                <br />
                {customer.billingAddress.country && (
                  <span>
                    {getCountryFlagEmoji(customer.billingAddress.country)}{' '}
                    {getCountryName(customer.billingAddress.country)}
                  </span>
                )}
              </div>
            )}
            {customer.vatNumber && (
              <div className="pt-1">
                <span className="text-muted-foreground">Tax ID: </span>
                {customer.vatNumber}
                <VatValidationBadge customer={customer} />
              </div>
            )}
          </div>
          <div className="flex items-center space-x-2">
            {actions}
            <Button variant="ghost" size="sm" className="p-0 h-auto" onClick={onEdit}>
              <Edit2 size={16} />
            </Button>
          </div>
        </div>
      </Card>
    </>
  )
}
