import { colors, radius, spaces } from '@md/foundation'
import { styled } from '@stitches/react'

export const Pill = styled('div', {
  display: 'inline-flex',
  justifyContent: 'center',
  alignItems: 'center',
  padding: '0.3em 0.5em',
  borderRadius: radius.radius3,
  gap: spaces.space4,

  variants: {
    color: {
      danger: {
        color: colors.danger3,
        backgroundColor: colors.danger9,
      },
      warning: {
        color: colors.warning3,
        backgroundColor: colors.warning9,
      },
      success: {
        color: colors.success3,
        backgroundColor: colors.success9,
      },
      neutral: {
        color: colors.neutral3,
        backgroundColor: colors.neutral9,
      },
      blue: {
        color: colors.purple3,
        backgroundColor: colors.purple9,
      },
    },
  },
})
