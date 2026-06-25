import { Button, Card, CardContent } from '@md/ui'
import { ExternalLink, ScrollText } from 'lucide-react'

import type { FunctionComponent } from 'react'

/**
 * The organization-wide audit log is a Meteroid Cloud / Enterprise feature.
 *
 * In the open-source edition this page renders an upgrade prompt instead of the
 * full audit trail. The per-entity activity timelines (on customers, invoices,
 * subscriptions, quotes, plans, …) remain fully functional — only this
 * org-wide aggregated view is gated.
 */
export const ActivityPage: FunctionComponent = () => {
  return (
    <div className="flex items-center justify-center w-full py-16">
      <Card className="max-w-xl w-full border-dashed">
        <CardContent className="flex flex-col items-center text-center gap-4 py-12 px-8">
          <div className="flex items-center justify-center h-12 w-12 rounded-full bg-muted text-muted-foreground">
            <ScrollText size={24} strokeWidth={1.5} />
          </div>
          <div className="space-y-3">
            <h3 className="text-lg font-semibold">Audit log is not available in this edition</h3>
            <p className="text-sm text-muted-foreground max-w-md">
              The organization-wide audit log — a searchable trail of every system and user action
              across your tenant — is part of Meteroid Cloud and Meteroid Enterprise edition.
            </p>
          </div>
          <div className="flex items-center gap-3 pt-2">
            <Button asChild variant="default">
              <a href="https://meteroid.com/pricing" target="_blank" rel="noreferrer">
                <span className="text-brand-foreground  inline-flex items-center">
                  Compare editions
                  <ExternalLink size={14} className="ml-1.5" />
                </span>
              </a>
            </Button>
            <Button asChild variant="outline">
              <a href="https://docs.meteroid.com" target="_blank" rel="noreferrer">
                Learn more
              </a>
            </Button>
          </div>
        </CardContent>
      </Card>
    </div>
  )
}
