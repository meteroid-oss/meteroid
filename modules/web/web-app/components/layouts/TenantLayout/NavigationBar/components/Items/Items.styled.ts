import { colors, spaces } from '@md/foundation'
import { styled } from '@stitches/react'

const NAVIGATION_BAR_WIDTH = 55

export const StyledItems = styled('ul', {
  maxWidth: NAVIGATION_BAR_WIDTH,
  width: '100%',
  display: 'flex',
  flexDirection: 'column',
  gap: spaces.space4,
})

export const ItemDivider = styled('hr', {
  width: `calc(100% - ${spaces.space5} * 2)`,
  height: 1,
  border: 'none',
  background: `linear-gradient(81deg, rgba(246,202,220,0) 0%, ${colors.neutral6} 49%, rgba(255,255,255,0) 100%)`,
  marginLeft: spaces.space5,
})
