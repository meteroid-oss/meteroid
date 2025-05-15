import { Flex } from '@ui/index'

import type { ReactNode } from 'react'

type EmptyStateSVGName = 'customers'

interface EmptyStateProps {
  title: string
  description: string
  imageName: EmptyStateSVGName
  actions?: ReactNode
}

export const EmptyState = ({ title, description, imageName, actions }: EmptyStateProps) => (
  <Flex direction="column" justify="center" align="center" className="h-full mx-auto max-w-[500px]">
    <img
      src={`/empty-state/${imageName}.svg`}
      alt={imageName}
      className="mb-5"
      width={150}
      height={150}
    />
    <div className="font-gravity text-center mb-1">{title}</div>
    <p className="text-center text-muted-foreground text-sm font-light mb-5">{description}</p>
    {actions}
  </Flex>
)
