import { colors, fontSizes, radius, spaces } from '@md/foundation'
import { styled } from '@stitches/react'

export const Info = styled('span', {
  color: colors.neutral9,
  fontSize: fontSizes.fontSize1,
  letterSpacing: '0.12px',
  marginTop: spaces.space2,
  animationName: 'fadeIn',
  animationDuration: '200ms',
  animationTimingFunction: 'ease-out',
})

export const Error = styled('span', {
  color: colors.danger9,
  fontSize: fontSizes.fontSize1,
  letterSpacing: '0.12px',
  animationName: 'fadeIn',
  animationDuration: '200ms',
  animationTimingFunction: 'ease-out',
})

export const StyledCheckboxFormItem = styled('div', {
  backgroundColor: colors.neutral1,
  borderRadius: radius.radius3,
  padding: `${spaces.space3} ${spaces.space5}`,
  border: `1px solid ${colors.neutral3}`,
})
