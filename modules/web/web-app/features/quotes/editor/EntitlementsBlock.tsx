import { Badge, Card, Collapsible, CollapsibleContent, CollapsibleTrigger } from '@md/ui'
import { ChevronRight } from 'lucide-react'

import { PendingEntitlementsPanel } from '@/features/entitlements/pending/PendingEntitlementsPanel'

import type { PendingEntitlementSpec } from '@/features/entitlements/creation/types'
import type { ComponentProps } from 'react'

interface EntitlementsBlockProps {
  selection: ComponentProps<typeof PendingEntitlementsPanel>['selection']
  pending: PendingEntitlementSpec[]
  onChange: (pending: PendingEntitlementSpec[]) => void
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
        <PendingEntitlementsPanel
          selection={selection}
          pending={pending}
          onChange={onChange}
          entityLabel="quote"
        />
      </CollapsibleContent>
    </Collapsible>
  </Card>
)
