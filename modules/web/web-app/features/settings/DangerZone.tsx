import { ConnectError } from '@connectrpc/connect'
import { useMutation } from '@connectrpc/connect-query'
import { Modal } from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { useState } from 'react'
import { Link, useNavigate } from 'react-router'
import { toast } from 'sonner'

import {
  TENANT_RESTORE_WINDOW_LABEL,
  useCancelTenantDeletion,
} from '@/features/settings/PendingDeletionTenants'
import {
  DangerAction,
  DangerZone as DangerZoneSection,
} from '@/features/settings/components/DangerZone'
import { useBasePath } from '@/hooks/useBasePath'
import { useFeaturesQuery } from '@/hooks/useEntitlement'
import { useOrganizationSlug } from '@/hooks/useOrganization'
import { useQuery } from '@/lib/connectrpc'
import { listConnectors } from '@/rpc/api/connectors/v1/connectors-ConnectorsService_connectquery'
import {
  deleteOrganization,
  getCurrentOrganizations,
} from '@/rpc/api/organizations/v1/organizations-OrganizationsService_connectquery'
import { TenantEnvironmentEnum } from '@/rpc/api/tenants/v1/models_pb'
import {
  activeTenant,
  deleteTenant,
  listTenants,
  updateTenant,
} from '@/rpc/api/tenants/v1/tenants-TenantsService_connectquery'
import { OrganizationUserRole } from '@/rpc/api/users/v1/models_pb'

export const DangerZone = () => {
  const navigate = useNavigate()
  const organizationSlug = useOrganizationSlug()
  const queryClient = useQueryClient()

  const currentTenant = useQuery(activeTenant).data?.tenant
  const tenants = useQuery(listTenants).data?.tenants ?? []
  const role = useQuery(getCurrentOrganizations).data?.role

  const basePath = useBasePath()
  const [promoted, setPromoted] = useState(false)
  const connectors = useQuery(listConnectors, {}).data?.connectors ?? []
  const productionTenantLimit = useFeaturesQuery().data?.productionTenantLimit
  const hasTestGateway = connectors.some(c => c.paymentCapabilities?.isTestGateway)

  const isAdmin = role === OrganizationUserRole.ADMIN
  const currentIsProduction = currentTenant?.environment === TenantEnvironmentEnum.PRODUCTION
  const hasProductionTenant = tenants.some(t => t.environment === TenantEnvironmentEnum.PRODUCTION)
  const isLastTenant = tenants.length <= 1

  const cancelDeletionMut = useCancelTenantDeletion({ openOnSuccess: true })

  const deleteTenantMut = useMutation(deleteTenant, {
    onSuccess: async ({ organizationDeleted }, { tenantId }) => {
      await queryClient.invalidateQueries()
      // The tenant (or whole org) is now hidden/inaccessible, so leave this route.
      if (organizationDeleted) {
        toast.success('Organization scheduled for deletion')
        window.location.replace('/')
      } else {
        toast.success('Tenant scheduled for deletion', {
          description: `You can restore it within ${TENANT_RESTORE_WINDOW_LABEL} from the tenant menu or Settings › General.`,
          action: { label: 'Undo', onClick: () => cancelDeletionMut.mutate({ tenantId }) },
        })
        navigate(`/${organizationSlug}`, { replace: true })
      }
    },
    onError: (err: Error) => toast.error(err.message ?? 'Failed to delete tenant'),
  })

  const promoteMut = useMutation(updateTenant, {
    onSuccess: async () => {
      await queryClient.invalidateQueries()
      setPromoted(true)
    },
    onError: (err: Error) =>
      toast.error(ConnectError.from(err).rawMessage || 'Failed to promote the tenant'),
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

  const productionCount = tenants.filter(
    t => t.environment === TenantEnvironmentEnum.PRODUCTION
  ).length
  const promoteDisabledReason = hasTestGateway
    ? 'Disconnect the test gateway in Integrations first: a production tenant cannot use it.'
    : productionTenantLimit !== undefined && productionCount >= productionTenantLimit
      ? `This organization can hold up to ${productionTenantLimit} production tenant(s).`
      : null

  const promote: DangerAction = {
    title: 'Promote to production',
    description:
      'Make this tenant your production tenant, with its customers, subscriptions and settings.',
    buttonLabel: 'Promote to production',
    disabledReason: promoteDisabledReason,
    onConfirm: () => promoteMut.mutate({ data: { environment: TenantEnvironmentEnum.PRODUCTION } }),
    confirmation: {
      header: 'Promote to production',
      message: (
        <div className="space-y-2">
          <p>&quot;{currentTenant.name}&quot; becomes a production tenant, in place:</p>
          <ul className="list-disc space-y-1 pl-5">
            <li>
              Everything in it stays, test customers and subscriptions included. Remove what should
              not be billed for real first.
            </li>
            <li>Its API keys become production keys.</li>
            <li>
              Payment providers stay connected as they are: replace test-mode keys with live ones.
              The test gateway must be disconnected first.
            </li>
            <li>Emails keep going to your team until you set delivery to Live.</li>
            <li>Production tenants cannot be deleted.</li>
          </ul>
          <p>Invoices finalized before the promotion are not counted as production revenue.</p>
        </div>
      ),
    },
  }

  const actions: DangerAction[] = [
    ...(currentIsProduction || currentTenant.environment === TenantEnvironmentEnum.SANDBOX
      ? []
      : [promote]),
    {
      title: 'Delete tenant',
      description: `This tenant and all its data will be permanently deleted. It can be restored within ${TENANT_RESTORE_WINDOW_LABEL}.`,
      buttonLabel: 'Delete tenant',
      disabledReason: tenantDisabledReason,
      onConfirm: () => deleteTenantMut.mutate({ tenantId: currentTenant.id }),
      confirmation: {
        header: isLastTenant ? 'Delete last tenant' : 'Delete tenant',
        message: isLastTenant
          ? `"${currentTenant.name}" is the last tenant, so the organization will be deleted too. All data is permanently removed. This cannot be undone.`
          : `"${currentTenant.name}" and all of its data will be permanently deleted. An admin can restore it within ${TENANT_RESTORE_WINDOW_LABEL} from the tenant menu or Settings › General.`,
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

  const settings = (tab: string) => `${basePath}/settings?tab=${tab}`
  return (
    <>
      <DangerZoneSection actions={actions} />
      <Modal
        visible={promoted}
        header={`"${currentTenant.name}" is now a production tenant`}
        onCancel={() => setPromoted(false)}
        onConfirm={() => setPromoted(false)}
        confirmText="Done"
        layout="vertical"
      >
        <div className="space-y-3 px-5 py-4 text-sm">
          <p className="text-muted-foreground">Before billing for real:</p>
          <ol className="list-decimal space-y-2 pl-5">
            <li>
              <Link className="underline" to={settings('general')}>
                Rename it
              </Link>{' '}
              if its name or slug still says it is a test workspace.
            </li>
            <li>
              <Link className="underline" to={settings('integrations')}>
                Replace test-mode payment and tax keys
              </Link>{' '}
              with live ones.
            </li>
            <li>
              <Link className="underline" to={settings('emails')}>
                Set customer emails to Live
              </Link>{' '}
              when you are ready for customers to receive them.
            </li>
            <li>Remove test customers and subscriptions that should not be billed.</li>
          </ol>
        </div>
      </Modal>
    </>
  )
}
