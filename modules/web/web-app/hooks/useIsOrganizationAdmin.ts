import { useQuery } from '@/lib/connectrpc'
import { getCurrentOrganizations } from '@/rpc/api/organizations/v1/organizations-OrganizationsService_connectquery'
import { OrganizationUserRole } from '@/rpc/api/users/v1/models_pb'

export const useIsOrganizationAdmin = () =>
  useQuery(getCurrentOrganizations).data?.role === OrganizationUserRole.ADMIN
