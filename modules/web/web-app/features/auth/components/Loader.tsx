import { Skeleton } from '@md/ui'
import { Fragment } from 'react'

export const Loader = () => (
  <Fragment>
    <Skeleton width={50} height={16} />
    <Skeleton
      height={44}
      style={{
        marginBottom: '0.75rem',
      }}
    />
    <Skeleton height={16} width={75} />
    <Skeleton
      height={44}
      style={{
        marginBottom: '0.75rem',
      }}
    />
    <Skeleton
      width={120}
      height={16}
      style={{
        marginTop: '1.25rem',
        marginBottom: '0.5rem',
      }}
    />
    <Skeleton height={44} />
  </Fragment>
)
