import { FlaskConical } from 'lucide-react'

import { useTenant } from '@/hooks/useTenant'
import { TenantEnvironmentEnum } from '@/rpc/api/tenants/v1/models_pb'

// On-screen previews leave the non-production banner out; say so where it would apply.
export const TestBannerNote = () => {
  const { tenant } = useTenant()
  if (!tenant || tenant.environment === TenantEnvironmentEnum.PRODUCTION) return null
  return (
    <p className="flex items-center gap-1.5 text-xs text-muted-foreground">
      <FlaskConical size={12} className="shrink-0" />
      Emails sent from this environment include a test banner, hidden in this preview.
    </p>
  )
}
