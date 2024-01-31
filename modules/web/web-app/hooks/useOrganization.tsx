import { useQuery } from '@/lib/connectrpc'
import { getInstance } from '@/rpc/api/instance/v1/instance-InstanceService_connectquery'

export const useOrganization = () => {
  const { data } = useQuery(getInstance)

  return {
    organization: data?.instance,
  }
}
