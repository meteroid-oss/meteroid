import { Button, Card, CardContent } from '@md/ui'
import { ExternalLink, ShieldCheck } from 'lucide-react'

import type { FunctionComponent } from 'react'

/**
 * Organization-wide security — MFA enforcement policy and the security activity
 * log — is a Meteroid Cloud / Enterprise feature.
 *
 * In the open-source edition this tab renders an upgrade prompt instead of the
 * full controls, mirroring how the org-wide audit log is gated.
 */
export const SecurityTab: FunctionComponent = () => {
  return (
    <div className="flex items-center justify-center w-full py-16">
      <Card className="max-w-xl w-full border-dashed">
        <CardContent className="flex flex-col items-center text-center gap-4 py-12 px-8">
          <div className="flex items-center justify-center h-12 w-12 rounded-full bg-muted text-muted-foreground">
            <ShieldCheck size={24} strokeWidth={1.5} />
          </div>
          <div className="space-y-3">
            <h3 className="text-lg font-semibold">Security is not available in this edition</h3>
            <p className="text-sm text-muted-foreground max-w-md">
              Enforcing two-factor authentication across your organization and reviewing the
              security activity log are part of Meteroid Cloud and Meteroid Enterprise edition.
            </p>
          </div>
          <div className="flex items-center gap-3 pt-2">
            <Button asChild variant="default">
              <a href="https://meteroid.com/pricing" target="_blank" rel="noreferrer">
                Compare editions
                <ExternalLink size={14} className="ml-1.5" />
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
