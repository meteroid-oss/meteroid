import { colors, radius } from '@md/foundation'
import * as CheckboxPrimitive from '@radix-ui/react-checkbox'
import { styled } from '@stitches/react'

export const StyledRoot = styled(CheckboxPrimitive.Root, {
  borderRadius: radius.radius1,
  transition: 'all 200ms ease-out',

  '&[data-state="unchecked"]': {
    backgroundColor: colors.neutral2,
    border: `1px solid ${colors.neutral8}`,

    '&:hover, &:focus': {
      backgroundColor: colors.neutral3,
    },

    '&:focus': {
      outline: '1px solid transparent',
      boxShadow: `0px 0px 0px 2px #F3F2F1, inset 0px 0px 1px 1.5px rgba(196, 196, 196, 0.5)`,
      borderColor: 'rgba(0, 0, 0, 0.22)',
    },
  },

  '&[data-state="checked"]': {
    backgroundColor: colors.secondary9,
    border: `1px solid ${colors.secondary3}`,

    '&:hover, &:focus': {
      backgroundColor: colors.secondary10,
    },

    '&:focus': {
      outline: '1px solid transparent',
      boxShadow: '0px 0px 0px 3px rgba(120, 115, 247, 0.2)',
      borderColor: 'rgba(0, 0, 0, 0.114)',
    },
  },
})
