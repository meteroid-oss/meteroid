import { colors, fontSizes, fontWeights, radius, spaces } from '@md/foundation'
import * as TogglePrimitive from '@radix-ui/react-toggle'
import { styled } from '@stitches/react'

export const StyledToggle = styled(TogglePrimitive.Root, {
  width: 'fit-content',
  height: 40,
  color: colors.primary12,
  padding: `0 ${spaces.space6}`,
  fontSize: fontSizes.fontSize2,
  fontWeight: fontWeights.medium,
  letterSpacing: '-0.1px',
  backgroundColor: colors.neutral3,
  borderRadius: radius.pill,
  cursor: 'pointer',
  lineHeight: 1,
  border: '1px solid transparent',
  transition:
    'background-color 0.2s ease, color 0.2s ease, border-color 0.2s ease, box-shadow 0.2s ease',

  '&:hover, &[data-state="on"]': {
    backgroundColor: colors.secondary9,
    color: colors.neutral1,
  },

  '&:focus': {
    outline: '1px solid transparent',
    boxShadow: '0px 0px 0px 3px rgba(0, 0, 0, 0.08)',
    borderColor: 'rgba(0, 0, 0, 0.22)',
  },
})
