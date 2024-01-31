import { colors, fontSizes, zIndices } from '@md/foundation'
import { keyframes, styled } from '@stitches/react'

export const StyledPageLoader = styled('div', {
  display: 'flex',
  justifyContent: 'center',
  alignItems: 'center',
  height: '100vh',
  width: '100vw',
  position: 'fixed',
  top: 0,
  left: 0,
  backgroundColor: colors.neutral1,
  zIndex: zIndices.zIndex5,
  color: colors.neutral9,
  fontSize: fontSizes.fontSize2,
  animation: 'fadeIn 200ms ease',
})

const bounceAnimation = keyframes({
  '0%, 10%, 20%, 30%, 50%, 70%, 80%, 90%, 100%': {
    transform: 'translateY(0)',
  },
  '15%': {
    transform: 'translateY(-15px)',
  },
  '40%, 60%': {
    transform: 'translateY(-7px)',
  },
})

export const AnimatedLogo = styled('div', {
  animation: `${bounceAnimation} 2s ease infinite`,
})
