import { useQuery } from '@/lib/connectrpc'
import { getCurrentOrganizations } from '@/rpc/api/organizations/v1/organizations-OrganizationsService_connectquery'

export const useIsExpressOrganization = (): boolean => {
  const { data } = useQuery(getCurrentOrganizations)
  return data?.organization?.isExpress ?? false
}

export const useExpressOrganizationState = (): { isExpress: boolean; isLoading: boolean } => {
  const { data, isLoading } = useQuery(getCurrentOrganizations)
  return {
    isExpress: data?.organization?.isExpress ?? false,
    isLoading,
  }
}
