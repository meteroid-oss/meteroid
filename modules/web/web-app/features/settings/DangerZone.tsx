import { useMutation } from '@connectrpc/connect-query'
import { useQueryClient } from '@tanstack/react-query'
import { useNavigate } from 'react-router-dom'
import { toast } from 'sonner'

import { DangerAction, DangerZone as DangerZoneSection } from '@/features/settings/components/DangerZone'
import { useOrganizationSlug } from '@/hooks/useOrganization'
import { useQuery } from '@/lib/connectrpc'
import {
  deleteOrganization,
  getCurrentOrganizations,
} from '@/rpc/api/organizations/v1/organizations-OrganizationsService_connectquery'
import { TenantEnvironmentEnum } from '@/rpc/api/tenants/v1/models_pb'
import {
  activeTenant,
  deleteTenant,
  listTenants,
} from '@/rpc/api/tenants/v1/tenants-TenantsService_connectquery'
import { OrganizationUserRole } from '@/rpc/api/users/v1/models_pb'

export const DangerZone = () => {
  const navigate = useNavigate()
  const organizationSlug = useOrganizationSlug()
  const queryClient = useQueryClient()

  const currentTenant = useQuery(activeTenant).data?.tenant
  const tenants = useQuery(listTenants).data?.tenants ?? []
  const role = useQuery(getCurrentOrganizations).data?.role

  const isAdmin = role === OrganizationUserRole.ADMIN
  const currentIsProduction = currentTenant?.environment === TenantEnvironmentEnum.PRODUCTION
  const hasProductionTenant = tenants.some(t => t.environment === TenantEnvironmentEnum.PRODUCTION)
  const isLastTenant = tenants.length <= 1

  const deleteTenantMut = useMutation(deleteTenant, {
    onSuccess: async ({ organizationDeleted }) => {
      await queryClient.invalidateQueries()
      toast.success(
        organizationDeleted
          ? 'Organization scheduled for deletion'
          : 'Tenant scheduled for deletion'
      )
      // The tenant (or whole org) is now hidden/inaccessible, so leave this route.
      if (organizationDeleted) {
        window.location.replace('/')
      } else {
        navigate(`/${organizationSlug}`, { replace: true })
      }
    },
    onError: (err: Error) => toast.error(err.message ?? 'Failed to delete tenant'),
  })

  const deleteOrgMut = useMutation(deleteOrganization, {
    onSuccess: () => {
      toast.success('Organization scheduled for deletion')
      window.location.replace('/')
    },
    onError: (err: Error) => toast.error(err.message ?? 'Failed to delete organization'),
  })

  if (!currentTenant) return null

  // Reasons a button is disabled (null = enabled). Deleting the last remaining
  // tenant is allowed, but takes the organization down with it (warned about in
  // the confirmation dialog).
  const tenantDisabledReason = !isAdmin
    ? 'Only organization admins can delete a tenant.'
    : currentIsProduction
      ? 'Production tenants cannot be deleted.'
      : null

  const organizationDisabledReason = !isAdmin
    ? 'Only organization admins can delete the organization.'
    : hasProductionTenant
      ? 'An organization with a production tenant cannot be deleted.'
      : null

  const actions: DangerAction[] = [
    {
      title: 'Delete tenant',
      description: 'This tenant and all its data will be permanently deleted. This can’t be undone.',
      buttonLabel: 'Delete tenant',
      disabledReason: tenantDisabledReason,
      onConfirm: () => deleteTenantMut.mutate({ tenantId: currentTenant.id }),
      confirmation: {
        header: isLastTenant ? 'Delete last tenant' : 'Delete tenant',
        message: isLastTenant
          ? `"${currentTenant.name}" is the last tenant, so the organization will be deleted too. All data is permanently removed. This cannot be undone.`
          : `"${currentTenant.name}" and all of its data will be permanently deleted. This cannot be undone.`,
        confirmLabel: isLastTenant ? 'Delete tenant & organization' : 'Delete tenant',
      },
    },
    {
      title: 'Delete organization',
      description:
        'This organization and every tenant under it will be permanently deleted. This can’t be undone.',
      buttonLabel: 'Delete organization',
      disabledReason: organizationDisabledReason,
      onConfirm: () => deleteOrgMut.mutate({}),
      confirmation: {
        header: 'Delete organization',
        message:
          'This organization and every tenant under it will be permanently deleted, with all data. This cannot be undone.',
        confirmLabel: 'Delete organization',
      },
    },
  ]

  return <DangerZoneSection actions={actions} />
}
