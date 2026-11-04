import { useQuery } from '@/lib/connectrpc'
import { getCurrentOrganizations } from '@/rpc/api/organizations/v1/organizations-OrganizationsService_connectquery'

/**
 * The calling user's effective permission codes in the current organization.
 *
 * Admins receive the full set from the backend, so `has('roles:manage')` is a
 * governance check that works for both admins and custom governance roles.
 * UX-only: server-side RBAC enforcement remains authoritative.
 */
export const usePermissions = () => {
  const { data, isLoading } = useQuery(getCurrentOrganizations)
  const permissions = data?.permissions ?? []
  return {
    permissions,
    isLoading,
    // Manage is a superset of view: holding `x:manage` implies `x:read`. Mirrors
    // the backend `RolePermissions::contains` implication, which stays authoritative.
    has: (code: string) => {
      if (permissions.includes(code)) return true
      if (code.endsWith(':read')) {
        return permissions.includes(`${code.slice(0, -':read'.length)}:manage`)
      }
      return false
    },
  }
}
