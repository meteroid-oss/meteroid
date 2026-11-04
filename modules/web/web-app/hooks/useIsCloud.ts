import { useQuery } from '@/lib/connectrpc'
import { getInstance } from '@/rpc/api/instance/v1/instance-InstanceService_connectquery'

/** true on Meteroid Cloud, false on a self-hosted instance, undefined while unknown. */
export const useIsCloud = (): boolean | undefined => {
  const { data } = useQuery(getInstance)
  return data?.isCloud
}
