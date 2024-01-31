import { colors, fontSizes, fontWeights, radius, spaces } from '@md/foundation'
import { styled } from '@stitches/react'

export const StyledAlert = styled('div', {
  position: 'relative',
  borderRadius: radius.radius3,
  padding: `${spaces.space5} ${spaces.space5} ${spaces.space5} ${spaces.space9}`,
  fontWeight: fontWeights.medium,
  fontSize: fontSizes.fontSize2,

  '&::before': {
    content: '',
    position: 'absolute',
    left: spaces.space5,
    top: spaces.space5,
    height: `calc(100% - ${spaces.space5} * 2)`,
    width: 4,
    borderRadius: radius.radius1,
  },

  defaultVariants: {
    variant: 'info',
  },

  variants: {
    variant: {
      success: {
        backgroundColor: colors.success3,
        color: colors.success11,

        '&::before': {
          backgroundColor: colors.success9,
        },
      },
      warning: {
        backgroundColor: colors.warning3,
        color: colors.warning11,

        '&::before': {
          backgroundColor: colors.warning9,
        },
      },
      danger: {
        backgroundColor: colors.danger3,
        color: colors.danger11,

        '&::before': {
          backgroundColor: colors.danger9,
        },
      },
      info: {
        backgroundColor: colors.secondary3,
        color: colors.secondary11,

        '&::before': {
          backgroundColor: colors.secondary9,
        },
      },
      note: {
        backgroundColor: colors.neutral3,
        color: colors.neutral11,

        '&::before': {
          backgroundColor: colors.neutral9,
        },
      },
    },
  },
})

export const Title = styled('b', {
  fontWeight: fontWeights.bold,
  color: 'inherit',
})
