import { ComponentProps, VariantProps } from '@stitches/react'

import { Dot, DotOne, DotThree, DotTwo, StyledDots } from '@ui/components/Dots/Dots.styled'

import type { FunctionComponent } from 'react'

export type DotsVariants = VariantProps<typeof Dot>
export type DotsProps = ComponentProps<typeof StyledDots> & DotsVariants

export const Dots: FunctionComponent<DotsProps> = ({ css, size, variant }) => (
  <StyledDots css={css}>
    <DotOne size={size} variant={variant} />
    <DotTwo size={size} variant={variant} />
    <DotThree size={size} variant={variant} />
  </StyledDots>
)
