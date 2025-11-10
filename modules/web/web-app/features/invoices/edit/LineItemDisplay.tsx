import { Badge, Button, cn } from '@md/ui'
import { ChevronDown, Edit, Trash2 } from 'lucide-react'
import { useState } from 'react'

import {
  OriginalLineItem,
  UpdateInvoiceLineSchema,
  UpdateInvoiceLineSchemaRegular,
  UpdateInvoiceLineSchemaWithSublines,
} from '@/lib/schemas/invoices'
import { formatCurrency, majorToMinorUnit } from '@/utils/numbers'

interface LineItemDisplayProps {
  item: UpdateInvoiceLineSchema | UpdateInvoiceLineSchemaWithSublines
  index: number
  currency: string
  onRemove: (index: number) => void
  onEdit: (index: number) => void
  isUsageBased: boolean
  originalItem?: OriginalLineItem
}

export const LineItemDisplay = ({
  item,
  index,
  currency,
  onRemove,
  onEdit,
  isUsageBased,
  originalItem,
}: LineItemDisplayProps) => {
  const [isExpanded, setIsExpanded] = useState(false)
  const itemWithValues = item as UpdateInvoiceLineSchemaRegular
  const hasSublines = originalItem?.subLineItems && originalItem.subLineItems.length > 0

  return (
    <div className="py-2 border-b last:border-b-0">
      <div className="flex justify-between items-start gap-2">
        <div className="flex-1 min-w-0">
          <div className="flex items-start gap-2">
            {hasSublines && (
              <ChevronDown
                size={12}
                className={cn(
                  'text-muted-foreground transition-transform cursor-pointer flex-shrink-0 mt-0.5',
                  isExpanded && 'rotate-180'
                )}
                onClick={() => setIsExpanded(!isExpanded)}
              />
            )}
            <div className="flex items-center gap-2 flex-wrap min-w-0">
              <div
                className={cn(
                  'text-[13px] font-medium break-words',
                  hasSublines && 'cursor-pointer'
                )}
                onClick={() => hasSublines && setIsExpanded(!isExpanded)}
              >
                {item.product}
              </div>
              {isUsageBased && (
                <Badge variant="outline" className="text-[10px] px-1.5 py-0 flex-shrink-0">
                  Usage-based
                </Badge>
              )}
            </div>
          </div>
          {item.startDate && item.endDate && (
            <div className={cn('text-[11px] text-muted-foreground mt-1', hasSublines && 'ml-4')}>
              {item.startDate.toLocaleDateString()} → {item.endDate.toLocaleDateString()}
            </div>
          )}
          {item.description && (
            <div
              className={cn('text-[11px] text-muted-foreground mt-1 italic', hasSublines && 'ml-4')}
            >
              {item.description}
            </div>
          )}
        </div>
        <div className="text-right flex items-center gap-2">
          <div>
            {!hasSublines &&
              itemWithValues.quantity !== null &&
              itemWithValues.quantity !== undefined &&
              itemWithValues.unitPrice !== null &&
              itemWithValues.unitPrice !== undefined && (
                <div className="text-[11px] text-muted-foreground">
                  {itemWithValues.quantity} ×{' '}
                  {formatCurrency(majorToMinorUnit(itemWithValues.unitPrice, currency), currency)}
                </div>
              )}
            <div className="text-[13px] font-medium">
              {hasSublines && originalItem
                ? formatCurrency(
                    originalItem.subLineItems && originalItem.subLineItems.length > 0
                      ? originalItem.subLineItems.reduce(
                          (sum: number, sub) => sum + Number(sub.total),
                          0
                        )
                      : Number(originalItem.subtotal),
                    currency
                  )
                : formatCurrency(
                    Number(itemWithValues.quantity ?? 0) *
                      Number(majorToMinorUnit(itemWithValues.unitPrice ?? 0, currency)),
                    currency
                  )}
            </div>
          </div>
          <div className="flex items-center gap-1">
            <Button
              type="button"
              variant="link"
              size="icon"
              onClick={() => onEdit(index)}
              className="h-6 w-6 p-0"
            >
              <Edit size={14} />
            </Button>
            <Button
              type="button"
              variant="link"
              size="icon"
              onClick={() => onRemove(index)}
              className="h-6 w-6 p-0 text-destructive hover:text-destructive"
            >
              <Trash2 size={14} />
            </Button>
          </div>
        </div>
      </div>

      {isExpanded && hasSublines && (
        <div className="mt-2 ml-4 pt-2 border-t space-y-1">
          {(originalItem.subLineItems ?? []).map(subItem => (
            <div key={subItem.id} className="flex justify-between items-center py-1">
              <span className="text-[11px] text-muted-foreground">{subItem.name}</span>
              <span className="text-[11px]">{formatCurrency(Number(subItem.total), currency)}</span>
            </div>
          ))}
        </div>
      )}
    </div>
  )
}
