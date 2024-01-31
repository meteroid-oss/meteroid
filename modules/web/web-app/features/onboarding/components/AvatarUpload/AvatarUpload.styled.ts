import { colors, fontSizes, fontWeights, radius } from '@md/foundation'
import { styled } from '@stitches/react'

export const StyledAvatarUpload = styled('div', {
  display: 'flex',
  justifyContent: 'center',
})

export const Placeholder = styled('button', {
  position: 'relative',
  width: 80,
  height: 80,
  backgroundColor: colors.secondary3,
  borderRadius: radius.round,
  display: 'flex',
  alignItems: 'center',
  justifyContent: 'center',
  fontSize: fontSizes.fontSize6,
  fontWeight: fontWeights.medium,
  userSelect: 'none',
  textTransform: 'uppercase',
  cursor: 'pointer',
  border: '1px solid transparent',
  transition: 'all 200ms ease-out',

  '&:focus': {
    outline: '1px solid transparent',
    boxShadow: '0px 0px 0px 3px rgba(120, 115, 247, 0.2)',
    borderColor: 'rgba(0, 0, 0, 0.114)',
  },
})

export const PlusContainer = styled('div', {
  position: 'absolute',
  bottom: 0,
  right: 0,
  height: 24,
  width: 24,
  display: 'flex',
  alignItems: 'center',
  justifyContent: 'center',
  backgroundColor: colors.white1,
  borderRadius: radius.round,
  boxShadow:
    '0px 4px 91px rgba(0, 0, 0, 0.15), 0px 1px 2px rgba(0, 0, 0, 0.07), 0px 2px 17px rgba(0, 0, 0, 0.07)',
})
