import { colors, fontSizes, fontWeights, spaces } from '@md/foundation'
import * as Dialog from '@radix-ui/react-dialog'
import { styled } from '@stitches/react'

import { Button } from '@ui/components/Button'

export const DialogContent = styled(Dialog.Content, {
  backgroundColor: colors.white1,
  boxShadow: '1px 0px 29px rgba(0, 0, 0, 0.1)',
  borderLeft: `1px solid ${colors.neutral4}`,
})

export const DialogOverylay = styled(Dialog.Overlay, {
  backgroundColor: colors.neutral1,
  backdropFilter: 'blur(4px)',
})

export const Header = styled('header', {
  position: 'relative',
  padding: `${spaces.space5} ${spaces.space9} ${spaces.space5} ${spaces.space9}`,
  marginBottom: spaces.space9,
  borderBottom: `1px solid ${colors.neutral4}`,
})

export const HeaderTitle = styled('h2', {
  fontSize: fontSizes.fontSize3,
  fontWeight: fontWeights.heavy,
  color: colors.primary12,
  letterSpacing: '-0.15px',
})

export const HeaderClose = styled(Button, {
  position: 'absolute',
  top: spaces.space4,
  right: spaces.space5,
})

export const Content = styled('div', {
  padding: `0 ${spaces.space9}`,
  // height: '100%',
  overflowY: 'auto',
})

export const Section = styled('div', {
  padding: `0 ${spaces.space9}`,
  overflowY: 'auto',
})

export const Footer = styled('footer', {
  display: 'flex',
  flexDirection: 'row',
  padding: spaces.space9,
  gap: spaces.space4,
})

export const twSidePanelStyles = {
  sidepanel: {
    base: `
          flex flex-col
          fixed
          inset-y-0
          h-screen
          z-40
        `,
    contents: `
          relative
          flex-1
          overflow-y-auto
        `,

    size: {
      medium: `w-screen max-w-md h-full`,
      large: `w-screen max-w-2xl h-full`,
      xlarge: `w-screen max-w-3xl h-full`,
      xxlarge: `w-screen max-w-4xl h-full`,
    },
    align: {
      left: `
            left-0
            data-open:animate-panel-slide-left-out
            data-closed:animate-panel-slide-left-in
          `,
      right: `
            right-0
            data-open:animate-panel-slide-right-out
            data-closed:animate-panel-slide-right-in
          `,
    },
    separator: `
          w-full
          h-px
          my-2
          bg-scale-300 dark:bg-scale-500
        `,
    overlay: `
          z-40
          fixed
          h-full w-full
          left-0
          top-0
          opacity-75
          data-closed:animate-fade-out-overlay-bg
          data-open:animate-fade-in-overlay-bg
        `,
  },
}
