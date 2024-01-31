import { colors, fontSizes, fontWeights, spaces } from '@md/foundation'
import { styled } from '@stitches/react'

import { TENANT_LAYOUT_INNER_MENU_WIDTH } from '@/components/layouts/TenantLayout/components/InnerMenu/InnerMenu.data'

export const StyledInnerMenu = styled('aside', {
  display: 'flex',
  flexDirection: 'column',
  width: TENANT_LAYOUT_INNER_MENU_WIDTH,
  backgroundColor: colors.mauveBackground,
  borderRight: `1px solid ${colors.mauve3}`,
})

export const Header = styled('header', {
  paddingTop: spaces.space9,
  paddingLeft: spaces.space6,
  lineHeight: 1,
})

export const HeaderTitle = styled('h2', {
  lineHeight: 1,
  fontWeight: fontWeights.medium,
  fontSize: fontSizes.fontSize4,
})
