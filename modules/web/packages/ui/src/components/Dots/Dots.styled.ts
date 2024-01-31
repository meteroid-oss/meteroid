import { colors, radius, spaces } from '@md/foundation'
import { keyframes, styled } from '@stitches/react'

export const StyledDots = styled('span', {
  display: 'flex',
})

export const Dot = styled('span', {
  borderRadius: radius.round,
  margin: `0 ${spaces.space2}`,

  variants: {
    variant: {
      light: {
        backgroundColor: colors.neutral1,
      },
      dark: {
        backgroundColor: colors.neutral12,
      },
    },
    size: {
      small: {
        width: 8,
        height: 8,
      },
      regular: {
        width: 10,
        height: 10,
      },
      large: {
        width: 14,
        height: 14,
      },
    },
  },

  defaultVariants: {
    variant: 'light',
    size: 'regular',
  },
})

export const bounce = keyframes({
  '0%': { opacity: 1 },
  '60%': { opacity: 0 },
  '100%': { opacity: 1 },
})

export const DotOne = styled(Dot, {
  animation: `${bounce} 1s infinite`,
  animationDelay: '0.1s',
})

export const DotTwo = styled(Dot, {
  animation: `${bounce} 1s infinite`,
  animationDelay: '0.3s',
})

export const DotThree = styled(Dot, {
  animation: `${bounce} 1s infinite`,
  animationDelay: '0.5s',
})
