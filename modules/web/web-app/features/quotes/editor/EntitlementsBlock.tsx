import { Badge, Card, Collapsible, CollapsibleContent, CollapsibleTrigger } from '@md/ui'
import { ChevronRight } from 'lucide-react'

import { SelectionGrantsEditor } from '@/features/entitlements/rows/DraftGrantsEditor'

import type { SelectionInput } from '@/features/entitlements/resolved/useResolvedEntitlements'
import type { GrantDraft } from '@/features/entitlements/rows/grantDraft'

interface EntitlementsBlockProps {
  selection: SelectionInput
  pending: GrantDraft[]
  onChange: (pending: GrantDraft[]) => void
}

export const EntitlementsBlock = ({ selection, pending, onChange }: EntitlementsBlockProps) => (
  <Card>
    <Collapsible defaultOpen={pending.length > 0}>
      <CollapsibleTrigger className="group flex w-full items-center gap-2 p-5 text-left">
        <ChevronRight className="h-4 w-4 shrink-0 text-muted-foreground transition-transform group-data-[state=open]:rotate-90" />
        <span className="flex items-center gap-2 text-base font-semibold">
          Entitlements
          {pending.length > 0 && (
            <Badge variant="secondary" size="sm">
              {pending.length} added
            </Badge>
          )}
        </span>
      </CollapsibleTrigger>
      <CollapsibleContent className="px-5 pb-5">
        <SelectionGrantsEditor selection={selection} drafts={pending} onChange={onChange} />
      </CollapsibleContent>
    </Collapsible>
  </Card>
)
