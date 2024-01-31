import { colors, fontSizes, fontWeights } from '@md/foundation'
import { styled } from '@stitches/react'

export const StyledNewsletterSubscription = styled('div', {
  fontSize: fontSizes.fontSize2,
  display: 'flex',
  flexDirection: 'column',

  'span:nth-of-type(1)': {
    fontWeight: fontWeights.medium,
    color: colors.primary11,
  },
  'span:nth-of-type(2)': {
    color: colors.neutral9,
  },
})
