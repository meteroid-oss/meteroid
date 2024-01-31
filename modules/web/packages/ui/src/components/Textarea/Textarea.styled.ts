import { colors, fontSizes, radius, spaces } from '@md/foundation'
import { styled } from '@stitches/react'

export const StyledTextarea = styled('textarea', {
  backgroundColor: colors.neutral1,
  color: colors.neutral12,
  fontSize: fontSizes.fontSize2,
  padding: `${spaces.space4} ${spaces.space6}`,
  borderRadius: radius.radius3,
  border: `1px solid hsla(0, 0%, 0%, 0.12)`,
  height: 94,
  resize: 'none',
  transition: 'box-shadow 200ms ease-out, color 200ms ease-out, border-color 200ms ease-out',

  '&::placeholder': {
    color: colors.neutral10,
  },

  '&:focus': {
    outline: '1px solid transparent',
    boxShadow: `0px 0px 0px 2px #F3F2F1, inset 0px 0px 1px 1.5px rgba(196, 196, 196, 0.5)`,
    borderColor: 'rgba(0, 0, 0, 0.22)',
  },
})
