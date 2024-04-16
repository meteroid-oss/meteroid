import { Button } from '@md/ui'
import { ComponentProps, ReactNode } from 'react'

type Props = ComponentProps<typeof Button> & {
  children?: ReactNode
}

export const CardAction = ({ children = 'Edit', ...props }: Props) => {
  return (
    <Button variant="secondary" className="py-1.5" {...props}>
      {children}
    </Button>
  )
}
