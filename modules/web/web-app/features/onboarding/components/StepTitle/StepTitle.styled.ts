import { fontSizes, fontWeights, spaces } from '@md/foundation'
import { styled } from '@stitches/react'

export const StyledStepTitle = styled('h1', {
  fontWeight: fontWeights.medium,
  fontSize: fontSizes.fontSize8,
  textAlign: 'center',
  lineHeight: '1.2',
  marginBottom: spaces.space10,
})
