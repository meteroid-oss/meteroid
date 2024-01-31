import { colors, fontSizes, fontWeights, spaces } from '@md/foundation'
import { styled } from '@stitches/react'

export const StyledTable = styled('table', {
  width: '100%',
  borderBottom: `1px solid ${colors.neutral3}`,
  // tableLayout: 'fixed',
})
export const StyledTHead = styled('thead', {
  borderTop: `1px solid ${colors.neutral3}`,
  borderBottom: `2px solid ${colors.neutral3}`,
})

export const StyledTh = styled('th', {
  padding: `${spaces.space6} 0`,
  lineHeight: 1,
  fontSize: fontSizes.fontSize2,
  fontWeight: fontWeights.medium,
  textAlign: 'left',
  color: colors.neutral10,

  '&:first-child': {
    paddingLeft: spaces.space5,
  },
})

export const StyledTBody = styled('tbody', {})

export const StyledTr = styled('tr', {
  '&:not(:last-child)': {
    borderBottom: `1px solid ${colors.neutral3}`,
  },

  variants: {
    hoverable: {
      true: {
        '&:hover': {
          backgroundColor: colors.neutral2,
        },
      },
    },
  },
})

export const StyledTd = styled('td', {
  padding: `${spaces.space4} 0`,
  fontSize: fontSizes.fontSize2,
  color: colors.neutral11,

  '&:first-child': {
    paddingLeft: spaces.space5,
  },
})
