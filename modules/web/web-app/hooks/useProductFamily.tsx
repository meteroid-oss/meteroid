import { disableQuery } from '@connectrpc/connect-query'

import { useQuery } from '@/lib/connectrpc'
import { useTypedParams } from '@/lib/utils/params'
import { getProductFamilyByLocalId } from '@/rpc/api/productfamilies/v1/productfamilies-ProductFamiliesService_connectquery'

export const useProductFamily = () => {
  const { familyLocalId } = useTypedParams()

  const productFamily = useQuery(
    getProductFamilyByLocalId,
    familyLocalId ? { localId: familyLocalId! } : disableQuery
  )

  return {
    productFamily: productFamily.data?.productFamily ?? undefined,
  }
}
