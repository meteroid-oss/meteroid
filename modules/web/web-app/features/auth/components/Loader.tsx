import { spaces } from '@md/foundation'
import { Skeleton } from '@ui2/components'
import { Fragment } from 'react'

export const Loader = () => (
  <Fragment>
    <Skeleton width={50} height={16} />
    <Skeleton
      height={44}
      style={{
        marginBottom: spaces.space5,
      }}
    />
    <Skeleton height={16} width={75} />
    <Skeleton
      height={44}
      style={{
        marginBottom: spaces.space5,
      }}
    />
    <Skeleton
      width={120}
      height={16}
      style={{
        marginTop: spaces.space7,
        marginBottom: spaces.space4,
      }}
    />
    <Skeleton height={44} />
  </Fragment>
)
