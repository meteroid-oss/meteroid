import { useMemo } from 'react'

import type { DimensionCombination } from '@/features/pricing/PricingFields'
import type { BillableMetric } from '@/rpc/api/billablemetrics/v1/models_pb'

export function useMatrixDimensions(billableMetric: BillableMetric | undefined): {
  dimensionHeaders: string[] | undefined
  validCombinations: DimensionCombination[] | undefined
} {
  return useMemo(() => {
    const seg = billableMetric?.segmentationMatrix
    if (!seg?.matrix) return { dimensionHeaders: undefined, validCombinations: undefined }

    let headers: string[] = []
    let combinations: DimensionCombination[] = []

    const matrix = seg.matrix
    if (matrix?.case === 'single') {
      const dim = matrix.value?.dimension
      headers = [dim?.key ?? '']
      combinations = (dim?.values ?? []).map((v: string) => ({
        dimension1: { key: headers[0], value: v },
      }))
    } else if (matrix?.case === 'double') {
      const d1 = matrix.value?.dimension1
      const d2 = matrix.value?.dimension2
      headers = [d1?.key ?? '', d2?.key ?? '']
      combinations = (d1?.values ?? []).flatMap((v1: string) =>
        (d2?.values ?? []).map((v2: string) => ({
          dimension1: { key: headers[0], value: v1 },
          dimension2: { key: headers[1], value: v2 },
        }))
      )
    } else if (matrix?.case === 'linked') {
      headers = [matrix.value.dimensionKey, matrix.value.linkedDimensionKey]
      combinations = Object.entries(matrix.value.values).flatMap(([k, v]) =>
        v.values.map(linkedV => ({
          dimension1: { key: headers[0], value: k },
          dimension2: { key: headers[1], value: linkedV },
        }))
      )
    }

    return { dimensionHeaders: headers, validCombinations: combinations }
  }, [billableMetric])
}
