import { disableQuery } from '@connectrpc/connect-query'

import { useQuery } from '@/lib/connectrpc'
import { useTypedParams } from '@/lib/utils/params'
import { getProductFamilyByExternalId } from '@/rpc/api/productfamilies/v1/productfamilies-ProductFamiliesService_connectquery'

export const useProductFamily = () => {
  const { familyExternalId } = useTypedParams()

  const productFamily = useQuery(
    getProductFamilyByExternalId,
    familyExternalId ? { externalId: familyExternalId! } : disableQuery
  )

  return {
    productFamily: productFamily.data ?? undefined,
  }
}
