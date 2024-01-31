import { Logo as LogoComponent, colors, fontSizes, radius, spaces } from '@md/foundation'
import { styled } from '@stitches/react'

export const StyledWizard = styled('nav', {
  padding: `${spaces.space9} 0`,
  width: '100%',
  display: 'flex',
  flexDirection: 'row',
  justifyContent: 'center',
  alignItems: 'center',
  animation: 'fadeIn 0.2s ease-in-out',
})

export const Items = styled('ul', {
  listStyleType: 'none',
  display: 'flex',
  flexDirection: 'row',
  alignItems: 'center',
  gap: spaces.space4,
})

export const Item = styled('li', {
  borderRadius: radius.radius3,
  height: 4,
  transition: 'all 0.2s ease-in-out',

  variants: {
    active: {
      true: {
        width: 48,
        backgroundColor: colors.primary9,
      },
      false: {
        width: 24,
        backgroundColor: colors.neutral4,
      },
    },
  },
})

export const Logo = styled(LogoComponent, {
  position: 'absolute',
  left: spaces.space9,
  top: spaces.space9,
})

export const StepCount = styled('span', {
  color: colors.neutral9,
  fontSize: fontSizes.fontSize2,
})
