import { Card } from '@md/ui'

import { SubscribedFor } from '@/features/subscriptions/SubscribedFor'

import type { ComponentProps } from 'react'

export const SubscribedForBlock = (props: ComponentProps<typeof SubscribedFor>) => (
  <Card className="flex flex-col gap-4 p-5">
    <h2 className="text-base font-semibold">Subscribed for</h2>
    <SubscribedFor {...props} />
  </Card>
)
