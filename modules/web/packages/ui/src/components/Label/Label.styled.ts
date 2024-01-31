import { colors, fontSizes, fontWeights } from '@md/foundation'
import * as LabelPrimitive from '@radix-ui/react-label'
import { styled } from '@stitches/react'

export const StyledLabel = styled(LabelPrimitive.Root, {
  color: colors.primary12,
  fontSize: fontSizes.fontSize2,
  fontWeight: fontWeights.medium,
})
