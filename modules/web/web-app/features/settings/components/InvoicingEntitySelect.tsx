import { Badge, Button } from '@md/ui'
import { PlusIcon } from 'lucide-react'
import { useState } from 'react'

import { Combobox } from '@/components/Combobox'
import { CreateInvoicingEntityDialog } from '@/features/settings/CreateInvoiceEntityDialog'
import { useInvoicingEntity } from '@/features/settings/hooks/useInvoicingEntity'
import { getCountryFlagEmoji } from '@/features/settings/utils'

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
  const { selectedEntityId, setSelectedEntityId, entities, isLoading } = useInvoicingEntity()
  const [createDialogOpen, setCreateDialogOpen] = useState(false)

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
            <div className="flex flex-row w-full">
              <div className="pr-2">{getCountryFlagEmoji(entity.country)}</div>
              <div>{entity.legalName}</div>
              <div className="flex-grow" />
              {entity.isDefault && (
                <Badge variant="primary" size="sm">
                  Default
                </Badge>
              )}
            </div>
          ),
          value: entity.id,
        }))}
        action={
          showCreateAction ? (
            <Button
              size="content"
              variant="ghost"
              hasIcon
              className="w-full border-none h-full"
              onClick={() => setCreateDialogOpen(true)}
            >
              <PlusIcon size="12" /> New invoicing entity
            </Button>
          ) : undefined
        }
      />

      {showCreateAction && (
        <CreateInvoicingEntityDialog
          open={createDialogOpen}
          setOpen={setCreateDialogOpen}
          setInvoicingEntity={setSelectedEntityId}
        />
      )}
    </>
  )
}
