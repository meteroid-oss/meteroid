import { useQueryClient } from '@tanstack/react-query'

import { useMutation } from '@/lib/connectrpc'
import { TenantStatusEnum } from '@/rpc/api/tenants/v1/models_pb'
import { resumeTenant } from '@/rpc/api/tenants/v1/tenants-TenantsService_connectquery'

interface TenantStatusBannerProps {
  tenantId: string | undefined
  status: TenantStatusEnum | undefined
}

/**
 * Shown when a tenant is paused (inactivity) or archived. Offers a Resume action
 * that reactivates it and refreshes the app so background processing resumes.
 */
export const TenantStatusBanner = ({ tenantId, status }: TenantStatusBannerProps) => {
  const queryClient = useQueryClient()
  const resumeMut = useMutation(resumeTenant, {
    onSuccess: () => queryClient.invalidateQueries(),
  })

  if (!tenantId || status === undefined || status === TenantStatusEnum.ACTIVE) {
    return null
  }

  const paused = status === TenantStatusEnum.PAUSED
  const label = paused
    ? 'This tenant is paused after 30 days of inactivity — background billing is suspended.'
    : 'This tenant is archived and pending deletion.'

  return (
    <div className="flex items-center justify-center gap-3 bg-amber-500 px-4 py-1.5 text-xs font-medium text-amber-950 pointer-events-auto">
      <span>{label}</span>
      <button
        type="button"
        disabled={resumeMut.isPending}
        onClick={() => resumeMut.mutate({ tenantId })}
        className="rounded bg-amber-950 px-2 py-0.5 font-semibold text-amber-50 hover:bg-amber-900 disabled:opacity-50"
      >
        {resumeMut.isPending ? 'Resuming…' : 'Resume tenant'}
      </button>
    </div>
  )
}
