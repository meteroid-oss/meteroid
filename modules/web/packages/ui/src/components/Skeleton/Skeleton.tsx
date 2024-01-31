import { colors } from '@md/foundation'
import SkeletonComponent from 'react-loading-skeleton'

import type { FunctionComponent } from 'react'
import type { SkeletonProps } from 'react-loading-skeleton'

export const Skeleton: FunctionComponent<SkeletonProps> = props => {
  return (
    <SkeletonComponent baseColor={colors.neutral3} highlightColor={colors.neutral4} {...props} />
  )
}
