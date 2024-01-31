import { styled } from '@stitches/react'

// export const SortableTh = styled('div', {
//   display: 'flex',
//   flexDirection: 'row',
//   // alignItems: 'center',
//   gap: spaces.space4,
//   cursor: 'pointer',
//   transition: 'color 200ms ease',

//   '&:hover': {
//     color: colors.neutral11,
//   },

//   'svg path': {
//     transition: 'fill 200ms ease',
//   },

//   'svg[data-type="chevron"]': {
//     transition: 'transform 150ms ease',
//     animation: 'fadeIn 100ms ease',
//   },

//   '&[data-sort="desc"] svg[data-type="chevron"]': {
//     transform: 'rotate(180deg)',
//   },
// })

const DefaultIndicatorIcon = () => (
  <svg width="6" height="10" viewBox="0 0 6 10" fill="none" xmlns="http://www.w3.org/2000/svg">
    <path d="M3 4.29296e-08L0 4L6 4L3 4.29296e-08Z" fill="currentColor" />
    <path d="M3 10L6 6L0 6L3 10Z" fill="currentColor" />
  </svg>
)

export const SortableDefaultIndicator = styled(DefaultIndicatorIcon, {
  animation: 'fadeIn 100ms ease',
})

export const SortableIndicatorContainer = styled('div', {
  width: 14,
  display: 'flex',
  justifyContent: 'center',
})

// export const StyledTable = styled(Table, {
//   width: '100%',

//   tbody: {
//     display: 'block',
//     overflow: 'auto',
//     width: '100%',
//   },

//   thead: {
//     width: '100%',
//     // display: 'table-header-group',
//   },

//   tr: {
//     display: 'table',
//     // tableLayout: 'fixed',
//     width: '100%',
//   },
// })

// export const StyledTh = styled(Table.th, {
//   variants: {
//     empty: {
//       true: {
//         width: 32,
//       },
//     },
//   },
// })

// export const StyledTd = styled(Table.td, {
//   variants: {
//     empty: {
//       true: {
//         width: 32,
//       },
//     },
//   },
// })
