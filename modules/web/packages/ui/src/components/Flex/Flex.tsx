import { CSSProperties, FunctionComponent, ReactNode } from 'react'

import { StyledFlex } from './Flex.styled'

interface FlexProps {
  fullWidth?: boolean
  fullHeight?: boolean
  direction?: 'row' | 'column' | 'row-reverse' | 'column-reverse'
  align?: 'flex-start' | 'flex-end' | 'center' | 'baseline' | 'stretch'
  justify?: 'flex-start' | 'flex-end' | 'center' | 'space-between' | 'space-around' | 'space-evenly'
  wrap?: 'nowrap' | 'wrap' | 'wrap-reverse'
  gap?: string
  style?: CSSProperties
  children: ReactNode | ReactNode[]
}

export const Flex: FunctionComponent<FlexProps> = ({
  fullHeight,
  fullWidth,
  direction,
  align,
  justify,
  wrap,
  gap,
  style,
  children,
}) => {
  return (
    <StyledFlex
      style={{
        height: fullHeight ? '100%' : undefined,
        width: fullWidth ? '100%' : undefined,
        flexDirection: direction,
        alignItems: align,
        justifyContent: justify,
        flexWrap: wrap,
        gap,
        ...style,
      }}
    >
      {children}
    </StyledFlex>
  )
}
