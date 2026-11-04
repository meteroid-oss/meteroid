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
    title="API Documentation"
    description="Explore and try the REST API used to build integrations and SDKs."
  >
    <div>
      <Button variant="secondary" size="sm" hasIcon asChild>
        <a href={`${env.meteroidRestApiUri}/scalar`} target="_blank" rel="noreferrer">
          Open API docs <ExternalLinkIcon size={14} />
        </a>
      </Button>
    </div>
  </SettingsSection>
)
