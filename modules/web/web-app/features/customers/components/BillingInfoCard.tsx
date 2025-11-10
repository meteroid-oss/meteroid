import { PartialMessage } from '@bufbuild/protobuf'
import { Button, Card } from '@md/ui'
import { Edit2 } from 'lucide-react'
import { ReactNode } from 'react'

import { getCountryFlagEmoji, getCountryName } from '@/features/settings/utils'
import { Customer } from '@/rpc/api/customers/v1/models_pb'

interface BillingInfoCardProps {
  customer: PartialMessage<Customer>
  onEdit: () => void
  title?: string
  actions?: ReactNode // Additional actions next to edit button
  cardVariant?: 'default' | 'accent' | 'accent2'
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
