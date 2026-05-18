import { Badge, Button } from '@md/ui'
import { Pencil, XIcon } from 'lucide-react'
import { useState } from 'react'

import { env } from '@/lib/env'

import { EntitlementSpecDialog } from './EntitlementSpecDialog'
import { PendingEntitlementSpec } from './types'

interface Props {
  initialEntitlements?: PendingEntitlementSpec[]
  submitLabel: string
  onBack: () => void
  onSubmit: (entitlements: PendingEntitlementSpec[]) => Promise<void>
  isSubmitting?: boolean
}

export function EntitlementCreationStep({
  initialEntitlements = [],
  submitLabel,
  onBack,
  onSubmit,
  isSubmitting,
}: Props) {
  const [pending, setPending] = useState<PendingEntitlementSpec[]>(initialEntitlements)
  const [dialogOpen, setDialogOpen] = useState(false)
  const [editingIndex, setEditingIndex] = useState<number | null>(null)

  const handleSubmit = () => onSubmit(pending)

  const openAdd = () => { setEditingIndex(null); setDialogOpen(true) }
  const openEdit = (i: number) => { setEditingIndex(i); setDialogOpen(true) }
  const handleClose = () => { setDialogOpen(false); setEditingIndex(null) }

  const handleAdd = (spec: PendingEntitlementSpec) => {
    if (editingIndex !== null) {
      setPending(prev => prev.map((e, i) => i === editingIndex ? spec : e))
    } else {
      setPending(prev => [...prev, spec])
    }
    handleClose()
  }

  if (!env.entitlementsEnabled) {
    return (
      <div className="flex gap-2 justify-end">
        <Button variant="secondary" type="button" onClick={onBack}>← Back</Button>
        <Button variant="primary" type="button" onClick={handleSubmit} disabled={isSubmitting}>
          {submitLabel}
        </Button>
      </div>
    )
  }

  const initialSpec = editingIndex !== null ? pending[editingIndex] : undefined

  return (
    <div className="space-y-6">
      <div>
        <h3 className="text-sm font-medium">
          Entitlements <span className="text-muted-foreground font-normal">(optional)</span>
        </h3>
        <p className="text-xs text-muted-foreground mt-1">
          Add entitlements that grant feature access to subscribers.
        </p>
      </div>

      {pending.length === 0 ? (
        <p className="text-sm text-muted-foreground">No entitlements configured.</p>
      ) : (
        <div className="border rounded-md divide-y">
          {pending.map((spec, i) => (
            <div key={i} className="flex items-center justify-between px-3 py-2 text-sm">
              <div className="flex items-center gap-2 flex-wrap">
                {!spec.featureId && (
                  <Badge variant="secondary" className="text-xs">New</Badge>
                )}
                <span className="font-medium">{spec.featureDisplayName}</span>
                <span className="text-muted-foreground text-xs">
                  {spec.featureType === 'boolean'
                    ? (spec.boolEnabled !== false ? 'Enabled' : 'Disabled')
                    : spec.limit ? `${spec.limit} / ${spec.resetPeriodType ?? 'cycle'}` : 'Unlimited'}
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
                  onClick={() => setPending(prev => prev.filter((_, idx) => idx !== i))}
                  className="p-1 text-muted-foreground hover:text-foreground hover:bg-muted rounded"
                >
                  <XIcon size={12} />
                </button>
              </div>
            </div>
          ))}
        </div>
      )}

      <Button variant="outline" type="button" onClick={openAdd}>
        + Add entitlement
      </Button>

      <EntitlementSpecDialog
        open={dialogOpen}
        onOpenChange={open => !open && handleClose()}
        initialSpec={initialSpec}
        onAdd={handleAdd}
        existingEntitlements={editingIndex !== null ? pending.filter((_, i) => i !== editingIndex) : pending}
      />

      <div className="flex gap-2 justify-end pt-2">
        <Button variant="secondary" type="button" onClick={onBack}>← Back</Button>
        <Button variant="primary" type="button" onClick={handleSubmit} disabled={isSubmitting}>
          {submitLabel}
        </Button>
      </div>
    </div>
  )
}
