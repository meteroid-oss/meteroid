import { useMutation } from '@connectrpc/connect-query'
import { Button } from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { format, formatDistanceToNow } from 'date-fns'
import { useNavigate } from 'react-router'
import { toast } from 'sonner'

import {
  SettingsRow,
  SettingsRowGroup,
  SettingsSection,
} from '@/features/settings/components/SettingsSection'
import { useOrganizationSlug } from '@/hooks/useOrganization'
import { useQuery } from '@/lib/connectrpc'
import { getCurrentOrganizations } from '@/rpc/api/organizations/v1/organizations-OrganizationsService_connectquery'
import {
  cancelTenantDeletion,
  listPendingDeletionTenants,
} from '@/rpc/api/tenants/v1/tenants-TenantsService_connectquery'
import { OrganizationUserRole } from '@/rpc/api/users/v1/models_pb'

export const TENANT_RESTORE_WINDOW_LABEL = '24 hours'

export const usePendingDeletionTenants = () => {
  const isAdmin = useQuery(getCurrentOrganizations).data?.role === OrganizationUserRole.ADMIN
  return useQuery(listPendingDeletionTenants, {}, { enabled: isAdmin }).data?.tenants ?? []
}

export const useCancelTenantDeletion = ({ openOnSuccess = false } = {}) => {
  const queryClient = useQueryClient()
  const navigate = useNavigate()
  const organizationSlug = useOrganizationSlug()
  return useMutation(cancelTenantDeletion, {
    onSuccess: async ({ tenant }) => {
      await queryClient.invalidateQueries()
      if (!tenant) return
      const open = () => navigate(`/${organizationSlug}/${tenant.slug}`)
      if (openOnSuccess) {
        toast.success(`"${tenant.name}" restored`)
        open()
      } else {
        toast.success(`"${tenant.name}" restored`, { action: { label: 'Open', onClick: open } })
      }
    },
    onError: (err: Error) => toast.error(err.message ?? 'Failed to restore tenant'),
  })
}

export const PendingDeletionTenants = () => {
  const tenants = usePendingDeletionTenants()
  const cancelMut = useCancelTenantDeletion()

  if (tenants.length === 0) return null

  return (
    <SettingsSection
      title="Pending deletion"
      description="These tenants are hidden and will be permanently deleted. You can restore them until the time shown."
    >
      <SettingsRowGroup>
        {tenants.map(({ tenant, deletedAt, restorableUntil }) => (
          <SettingsRow
            key={tenant?.id}
            title={tenant?.name}
            description={`Deleted ${formatDistanceToNow(new Date(deletedAt), { addSuffix: true })} · restorable until ${format(new Date(restorableUntil), 'PPp')}`}
          >
            <Button
              variant="secondary"
              size="sm"
              disabled={cancelMut.isPending}
              onClick={() => tenant && cancelMut.mutate({ tenantId: tenant.id })}
            >
              Cancel deletion
            </Button>
          </SettingsRow>
        ))}
      </SettingsRowGroup>
    </SettingsSection>
  )
}
