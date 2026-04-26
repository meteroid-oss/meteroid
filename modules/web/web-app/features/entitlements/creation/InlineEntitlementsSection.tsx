import { Badge, Button } from '@md/ui'
import { Pencil, XIcon } from 'lucide-react'
import { useState } from 'react'

import { env } from '@/lib/env'

import { groupByProduct } from '../utils'

import { EntitlementSpecDialog } from './EntitlementSpecDialog'

import type { PendingEntitlementSpec } from './types'

interface Props {
  entitlements: PendingEntitlementSpec[]
  onChange: (entitlements: PendingEntitlementSpec[]) => void
}

export function InlineEntitlementsSection({ entitlements, onChange }: Props) {
  const [dialogOpen, setDialogOpen] = useState(false)
  const [editingIndex, setEditingIndex] = useState<number | null>(null)

  if (!env.entitlementsEnabled) return null

  const openAdd = () => {
    setEditingIndex(null)
    setDialogOpen(true)
  }
  const openEdit = (i: number) => {
    setEditingIndex(i)
    setDialogOpen(true)
  }
  const handleClose = () => {
    setDialogOpen(false)
    setEditingIndex(null)
  }

  const handleAdd = (spec: PendingEntitlementSpec) => {
    if (editingIndex !== null) {
      onChange(entitlements.map((e, i) => (i === editingIndex ? spec : e)))
    } else {
      onChange([...entitlements, spec])
    }
    handleClose()
  }

  const initialSpec = editingIndex !== null ? entitlements[editingIndex] : undefined

  const groups = groupByProduct(entitlements, spec =>
    spec.productId ? { id: spec.productId, name: spec.productName ?? spec.productId } : undefined
  )

  return (
    <div className="space-y-3 pt-4 border-t border-border mt-4">
      <div className="flex items-center justify-between">
        <div>
          <h3 className="text-sm font-medium">Entitlements</h3>
          <p className="text-xs text-muted-foreground">Feature access granted to subscribers.</p>
        </div>
        <Button variant="outline" size="sm" type="button" onClick={openAdd}>
          + Add
        </Button>
      </div>

      {entitlements.length > 0 && (
        <div className="space-y-2">
          {groups.map(group => (
            <div key={group.id ?? '__general__'}>
              <p className="text-xs font-semibold uppercase tracking-wide text-muted-foreground mb-1 px-1">
                {group.name}
              </p>
              <div className="border rounded-md divide-y">
                {group.items.map(spec => {
                  const i = entitlements.indexOf(spec)
                  return (
                    <div key={i} className="flex items-center justify-between px-3 py-2 text-sm">
                      <div className="flex items-center gap-2 flex-wrap">
                        {!spec.featureId && (
                          <Badge variant="secondary" className="text-xs">
                            New
                          </Badge>
                        )}
                        <span className="font-medium">{spec.featureDisplayName}</span>
                        <span className="text-muted-foreground text-xs">
                          {spec.featureType === 'boolean'
                            ? spec.boolEnabled !== false
                              ? 'Enabled'
                              : 'Disabled'
                            : spec.limit
                              ? `${spec.limit} / ${spec.resetPeriodType ?? 'cycle'}`
                              : 'Unlimited'}
                        </span>
                      </div>
                      <div className="flex items-center gap-1 ml-2 shrink-0">
                        <button
                          type="button"
                          onClick={() => openEdit(i)}
                          className="p-1 text-muted-foreground hover:text-foreground hover:bg-muted rounded"
                        >
                          <Pencil size={12} />
                        </button>
                        <button
                          type="button"
                          onClick={() => onChange(entitlements.filter((_, idx) => idx !== i))}
                          className="p-1 text-muted-foreground hover:text-foreground hover:bg-muted rounded"
                        >
                          <XIcon size={12} />
                        </button>
                      </div>
                    </div>
                  )
                })}
              </div>
            </div>
          ))}
        </div>
      )}

      <EntitlementSpecDialog
        open={dialogOpen}
        onOpenChange={open => !open && handleClose()}
        initialSpec={initialSpec}
        onAdd={handleAdd}
        existingEntitlements={editingIndex !== null ? entitlements.filter((_, i) => i !== editingIndex) : entitlements}
      />
    </div>
  )
}
