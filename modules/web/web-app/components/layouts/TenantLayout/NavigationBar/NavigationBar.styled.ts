import { colors, spaces, zIndices } from '@md/foundation'
import { styled } from '@stitches/react'

import { NAVIGATION_BAR_WIDTH } from './NavigationBar.data'
export const StyledNavigationBar = styled('nav', {
  width: NAVIGATION_BAR_WIDTH,
  backgroundColor: colors.white1,
  display: 'flex',
  flexDirection: 'column',
  alignItems: 'center',
  justifyContent: 'space-between',
  borderRight: `1px solid ${colors.mauve3}`,
  padding: `${spaces.space7} 0`,
  zIndex: zIndices.zIndex9,
})
