import { useQueryClient } from '@tanstack/react-query'
import { Button } from '@ui/index'
import { useState } from 'react'
import { Link, useNavigate } from 'react-router'

import { CreateTenantModal } from '@/features/tenants/CreateTenant'
import { useBasePath } from '@/hooks/useBasePath'
import { useOrganizationSlug } from '@/hooks/useOrganization'
import { useQuery } from '@/lib/connectrpc'
import { TenantEnvironmentEnum } from '@/rpc/api/tenants/v1/models_pb'
import { listTenants } from '@/rpc/api/tenants/v1/tenants-TenantsService_connectquery'

const environmentLabels: Record<TenantEnvironmentEnum, string> = {
  [TenantEnvironmentEnum.PRODUCTION]: 'Production',
  [TenantEnvironmentEnum.STAGING]: 'Staging',
  [TenantEnvironmentEnum.QA]: 'QA',
  [TenantEnvironmentEnum.DEVELOPMENT]: 'Development',
  [TenantEnvironmentEnum.SANDBOX]: 'Sandbox',
  [TenantEnvironmentEnum.DEMO]: 'Demo',
}

/**
 * Full-width bar across the top of the app while working in a non-production environment, with
 * the way out: integrate (API keys) or go live (the production environment, or create one).
 * Rendered `fixed`. While it is on the page, the tenant layout's wrapper sets
 * `--env-banner-height` (42px, see router/tenant) and the sidebar and content shift down by it.
 */
export const EnvironmentBanner = ({
  environment,
}: {
  environment: TenantEnvironmentEnum | undefined
}) => {
  const basePath = useBasePath()
  const org = useOrganizationSlug()
  const navigate = useNavigate()
  const queryClient = useQueryClient()
  const tenants = useQuery(listTenants).data?.tenants ?? []
  const [creating, setCreating] = useState(false)

  if (environment === undefined || environment === TenantEnvironmentEnum.PRODUCTION) {
    return null
  }

  const label = environmentLabels[environment] ?? 'Test'
  const production = tenants.find(t => t.environment === TenantEnvironmentEnum.PRODUCTION)

  const goLive = () => {
    if (production) {
      // Same as switching tenants from the sidebar: drop the current tenant's cached data.
      queryClient.invalidateQueries()
      navigate(`/${org}/${production.slug}`)
    } else {
      setCreating(true)
    }
  }

  return (
    <div
      data-env-banner
      role="status"
      className="fixed inset-x-0 top-0 z-30 grid h-(--env-banner-height) grid-cols-[minmax(0,1fr)_minmax(0,710px)_minmax(0,1fr)] items-center gap-4 bg-[#15434c] pl-4 pr-2 text-sm font-medium tracking-[-0.14px] text-[#86f5ff]"
    >
      <span className="truncate">{label}</span>
      <p className="hidden truncate text-center md:block">
        This is your {label.toLowerCase()} environment. When you&apos;re ready, integrate Meteroid
        into your application.
      </p>
      <div className="col-start-3 flex items-center justify-end gap-2">
        <Button
          asChild
          variant="ghost"
          size="content"
          className="h-7 rounded-[4px] bg-white/12 px-2.5 text-[13px] tracking-[-0.13px] text-[#fafafa] hover:bg-white/20 hover:text-[#fafafa]"
        >
          <Link to={`${basePath}/settings/api-keys`}>Integrate</Link>
        </Button>
        <Button
          variant="ghost"
          size="content"
          onClick={goLive}
          className="h-7 rounded-[4px] bg-[#fafafa] px-2.5 text-[13px] tracking-[-0.13px] text-[#0a0a0a] hover:bg-white/85 hover:text-[#0a0a0a]"
        >
          Go live
        </Button>
      </div>
      <CreateTenantModal open={creating} onOpenChange={setCreating} />
    </div>
  )
}
