import { Badge, Button } from '@md/ui'
import { PlusIcon } from 'lucide-react'
import { useState } from 'react'

import { Combobox } from '@/components/Combobox'
import { Slot, slotItems } from '@/edition'
import { useInvoicingEntity } from '@/features/settings/hooks/useInvoicingEntity'
import { getCountryFlagEmoji } from '@/features/settings/utils'
import { useFeaturesQuery } from '@/hooks/useEntitlement'
import { useIsExpressOrganization } from '@/hooks/useIsExpressOrganization'

const canCreateEntities = () => slotItems('invoicingEntitySelect.createDialog').length > 0

interface InvoicingEntitySelectProps {
  className?: string
  placeholder?: string
  showCreateAction?: boolean
}

export const InvoicingEntitySelect = ({
  className = 'max-w-[300px]',
  placeholder = 'Select',
  showCreateAction = true,
}: InvoicingEntitySelectProps) => {
  const isExpress = useIsExpressOrganization()
  const { selectedEntityId, setSelectedEntityId, entities, isLoading } = useInvoicingEntity()
  const [createDialogOpen, setCreateDialogOpen] = useState(false)
  const limit = useFeaturesQuery().data?.invoicingEntityLimit
  const atLimit = limit !== undefined && entities.length >= limit
  const showCreate = showCreateAction && canCreateEntities()

  if (isExpress) return null

  if (isLoading) {
    return <div className={`${className} h-10 bg-muted animate-pulse rounded-md`} />
  }

  return (
    <>
      <Combobox
        placeholder={placeholder}
        className={className}
        value={selectedEntityId}
        onChange={setSelectedEntityId}
        options={entities.map(entity => ({
          label: (
            <div className="flex w-full flex-row items-center gap-2">
              <span className="shrink-0">{getCountryFlagEmoji(entity.country)}</span>
              <span className="truncate">{entity.legalName}</span>
              {entity.isDefault && (
                <Badge variant="outline" size="sm" className="ml-auto shrink-0 font-normal">
                  Default
                </Badge>
              )}
            </div>
          ),
          value: entity.id,
        }))}
        action={
          showCreate ? (
            atLimit ? (
              <p className="px-2 py-1.5 text-xs text-muted-foreground">
                Your plan allows up to {limit} invoicing {limit === 1 ? 'entity' : 'entities'} per
                tenant
              </p>
            ) : (
              <Button
                size="content"
                variant="ghost"
                hasIcon
                className="w-full border-none h-full"
                onClick={() => setCreateDialogOpen(true)}
              >
                <PlusIcon size="12" /> New invoicing entity
              </Button>
            )
          ) : undefined
        }
      />

      {showCreate && (
        <Slot
          name="invoicingEntitySelect.createDialog"
          open={createDialogOpen}
          setOpen={setCreateDialogOpen}
          setInvoicingEntity={setSelectedEntityId}
        />
      )}
    </>
  )
}
