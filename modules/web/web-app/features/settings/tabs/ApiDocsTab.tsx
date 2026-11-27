import { Button } from '@md/ui'
import { ExternalLinkIcon } from 'lucide-react'
import { FunctionComponent } from 'react'

import { SettingsSection } from '@/features/settings/components/SettingsSection'
import { env } from '@/lib/env'

// Not embedded: the reference writes its navigation to the browser history, which closing
// settings would then step back through.
export const ApiDocsTab: FunctionComponent = () => (
  <SettingsSection
    divided={false}
    description="Explore and try the REST API used to build integrations and SDKs. v1 is frozen: new integrations use v2."
  >
    <div className="flex flex-wrap items-center gap-2">
      <Button variant="secondary" size="sm" hasIcon asChild>
        <a href={`${env.meteroidRestApiUri}/scalar`} target="_blank" rel="noreferrer">
          Open API v2 docs <ExternalLinkIcon size={14} />
        </a>
      </Button>
      <Button variant="ghost" size="sm" hasIcon asChild>
        <a href={`${env.meteroidRestApiUri}/scalar/v1`} target="_blank" rel="noreferrer">
          API v1 (legacy) <ExternalLinkIcon size={14} />
        </a>
      </Button>
    </div>
  </SettingsSection>
)
