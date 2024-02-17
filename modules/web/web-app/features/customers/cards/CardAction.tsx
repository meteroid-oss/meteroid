import { ButtonAlt } from '@ui/components'
import { ComponentProps, ReactNode } from 'react'

type Props = ComponentProps<typeof ButtonAlt> & {
  children?: ReactNode
}

export const CardAction = ({ children = 'Edit', ...props }: Props) => {
  return (
    <ButtonAlt type="alternative" className="py-1.5" {...props}>
      {children}
    </ButtonAlt>
  )
}
