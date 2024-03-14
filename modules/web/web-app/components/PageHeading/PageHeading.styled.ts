import { colors, fontSizes, fontWeights, spaces } from '@md/foundation'
import { styled } from '@stitches/react'

export const StyledPageHeading = styled('h1', {
  fontSize: fontSizes.fontSize8,
  fontWeight: fontWeights.bold,
  lineHeight: 1,
})

export const Count = styled('span', {
  display: 'inline-block',
  fontSize: fontSizes.fontSize4,
  fontWeight: fontWeights.medium,
  lineHeight: 1,
  color: colors.neutral9,
  marginLeft: spaces.space3,
})
