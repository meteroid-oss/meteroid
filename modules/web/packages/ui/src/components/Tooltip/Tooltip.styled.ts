import { colors, fontSizes, fontWeights, radius, spaces } from '@md/foundation'
import * as TooltipPrimitive from '@radix-ui/react-tooltip'
import { keyframes, styled } from '@stitches/react'

export const StyledTooltipArrow = styled(TooltipPrimitive.Arrow, {
  '.polygon': {
    fill: 'var(--colors-slate1)',

    '.dark &': {
      fill: 'var(--colors-gray1)',
    },
  },
})

const TooltipAnimation = keyframes({
  '0%': { opacity: 0, transform: 'translateX(-4px)' },
  '100%': { opacity: 1, transform: 'translateY(0)' },
})

export const StyledTooltipContent = styled(TooltipPrimitive.Content, {
  backgroundColor: colors.mauve12,
  borderRadius: radius.radius2,
  padding: `${spaces.space2} ${spaces.space5}`,
  fontSize: fontSizes.fontSize2,
  fontWeight: fontWeights.medium,
  color: colors.mauveDark12,
  animation: `${TooltipAnimation} 150ms ease-out`,
})
