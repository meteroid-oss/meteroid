import { colors, fontSizes, fontWeights, radius, spaces } from '@md/foundation'
import { styled } from '@stitches/react'

export const StyledButton = styled('button', {
  display: 'flex',
  alignItems: 'center',
  justifyContent: 'center',
  gap: spaces.space4,
  lineHeight: 1,
  cursor: 'pointer',
  fontSize: fontSizes.fontSize2,
  fontWeight: fontWeights.medium,
  borderRadius: radius.radius3,
  border: '1px solid transparent',
  transition:
    'box-shadow 200ms ease-out, background-color 200ms ease-out, color 200ms ease-out, border-color 200ms ease-out',

  defaultVariants: {
    variant: 'primary',
    rounded: false,
    size: 'medium',
  },
  variants: {
    variant: {
      primary: {
        backgroundColor: colors.primary9,
        color: colors.white1,

        '&:hover, &:focus': {
          backgroundColor: colors.primary11,
        },

        '&:focus': {
          outline: '1px solid transparent',
          boxShadow: '0px 0px 0px 3px rgba(8, 7, 6, 0.2)',
        },
      },
      secondary: {
        backgroundColor: colors.secondary9,
        color: colors.white1,

        '&:hover, &:focus': {
          backgroundColor: colors.secondary10,
        },

        '&:focus': {
          outline: '1px solid transparent',
          boxShadow: '0px 0px 0px 3px rgba(120, 115, 247, 0.2)',
          borderColor: 'rgba(0, 0, 0, 0.114)',
        },
      },
      tertiary: {
        backgroundColor: colors.white1,
        color: colors.neutral12,
        borderColor: 'hsla(0, 0%, 0%, 0.12)',

        '&:hover, &:focus': {
          backgroundColor: colors.neutral1,
        },

        '&:focus': {
          outline: '1px solid transparent',
          boxShadow: '0px 0px 0px 3px rgba(0, 0, 0, 0.08)',
          borderColor: 'rgba(0, 0, 0, 0.22)',
        },
      },
      muted: {
        backgroundColor: colors.neutral3,
        color: colors.neutral12,

        '&:hover, &:focus': {
          backgroundColor: colors.neutral4,
        },

        '&:focus': {
          outline: '1px solid transparent',
          boxShadow: '0px 0px 0px 3px rgba(0, 0, 0, 0.08)',
          borderColor: 'rgba(0, 0, 0, 0.114)',
        },
      },
      success: {
        backgroundColor: colors.success9,
        color: colors.white1,

        '&:hover, &:focus': {
          backgroundColor: colors.success10,
        },

        '&:focus': {
          outline: '1px solid transparent',
          boxShadow: '0px 0px 0px 3px rgba(1, 159, 174, 0.2)',
          borderColor: 'rgba(0, 0, 0, 0.114)',
        },
      },
      warning: {
        backgroundColor: colors.warning9,
        color: colors.primary9,

        '&:hover, &:focus': {
          backgroundColor: colors.warning10,
        },

        '&:focus': {
          outline: '1px solid transparent',
          boxShadow: '0px 0px 0px 3px rgba(173, 113, 2, 0.2)',
          borderColor: 'rgba(0, 0, 0, 0.114)',
        },
      },
      danger: {
        backgroundColor: colors.danger9,
        color: colors.white1,

        '&:hover, &:focus': {
          backgroundColor: colors.danger10,
        },

        '&:focus': {
          outline: '1px solid transparent',
          boxShadow: '0px 0px 0px 3px rgba(255, 94, 60, 0.2)',
          borderColor: 'rgba(0, 0, 0, 0.114)',
        },
      },
    },
    rounded: {
      true: {
        borderRadius: radius.radius6,
      },
    },
    disabled: {
      true: {
        pointerEvents: 'none',
        opacity: 0.5,
      },
    },
    transparent: {
      true: {
        backgroundColor: 'transparent',
        color: colors.primary9,
      },
    },
    size: {
      tiny: {
        height: 28,
        padding: `${spaces.space2} 10px`,
      },
      small: {
        height: 32,
        padding: `${spaces.space3} ${spaces.space5}`,
      },
      medium: {
        height: 36,
        padding: `${spaces.space4} ${spaces.space6}`,
      },
      large: {
        height: 40,
        padding: `10px ${spaces.space6}`,
      },
    },
    icon: {
      true: {
        padding: '0 !important',
      },
    },
  },
  compoundVariants: [
    {
      variant: 'primary',
      transparent: true,
      css: {
        color: colors.primary9,

        '&:hover, &:focus': {
          color: colors.primary11,
          backgroundColor: colors.primary1,
        },
      },
    },
    {
      variant: 'secondary',
      transparent: true,
      css: {
        color: colors.secondary9,

        '&:hover, &:focus': {
          color: colors.secondary10,
          backgroundColor: colors.secondary1,
        },

        '&:focus': {
          borderColor: colors.secondary6,
        },
      },
    },
    {
      variant: 'tertiary',
      transparent: true,
      css: {
        color: colors.neutral12,
        borderColor: 'transparent',

        '&:hover, &:focus': {
          color: colors.neutral12,
          backgroundColor: colors.neutral1,
        },
      },
    },
    {
      variant: 'muted',
      transparent: true,
      css: {
        color: colors.neutral12,

        '&:hover, &:focus': {
          color: colors.neutral12,
          backgroundColor: colors.neutral1,
        },

        '&:focus': {
          borderColor: colors.neutral6,
        },
      },
    },
    {
      variant: 'success',
      transparent: true,
      css: {
        color: colors.success9,

        '&:hover, &:focus': {
          color: colors.success10,
          backgroundColor: colors.success1,
        },

        '&:focus': {
          borderColor: colors.success6,
        },
      },
    },
    {
      variant: 'warning',
      transparent: true,
      css: {
        color: colors.warning9,

        '&:hover, &:focus': {
          color: colors.warning10,
          backgroundColor: colors.warning1,
        },

        '&:focus': {
          borderColor: colors.warning6,
        },
      },
    },
    {
      variant: 'danger',
      transparent: true,
      css: {
        color: colors.danger9,

        '&:hover, &:focus': {
          color: colors.danger10,
          backgroundColor: colors.danger1,
        },

        '&:focus': {
          boxShadow: '0px 0px 0px 3px rgba(255, 94, 60, 0.2)',
          borderColor: colors.danger6,
        },
      },
    },
    {
      icon: true,
      size: 'tiny',
      css: {
        width: 28,
        height: 28,
        minWidth: 28,
        minHeight: 28,
      },
    },
    {
      icon: true,
      size: 'small',
      css: {
        width: 32,
        height: 32,
        minWidth: 32,
        minHeight: 32,
      },
    },
    {
      icon: true,
      size: 'medium',
      css: {
        width: 36,
        height: 36,
        minWidth: 36,
        minHeight: 36,
      },
    },
    {
      icon: true,
      size: 'large',
      css: {
        width: 40,
        height: 40,
        minWidth: 40,
        minHeight: 40,
      },
    },
  ],
})
