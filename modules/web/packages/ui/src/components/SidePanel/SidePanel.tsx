import { XMarkIcon } from '@md/icons'
import * as Dialog from '@radix-ui/react-dialog'

import { Button } from '@ui/components/Button'

import {
  DialogContent,
  DialogOverylay,
  Footer,
  Header,
  HeaderClose,
  HeaderTitle,
  Content as StyledContent,
  Section as StyledSection,
  twSidePanelStyles,
} from './SidePanel.styled'
export type SidePanelProps = RadixProps & CustomProps

interface RadixProps
  extends Dialog.DialogProps,
    Pick<
      Dialog.DialogContentProps,
      | 'onOpenAutoFocus'
      | 'onCloseAutoFocus'
      | 'onEscapeKeyDown'
      | 'onPointerDownOutside'
      | 'onInteractOutside'
    > {}

interface CustomProps {
  id?: string | undefined
  className?: string
  children?: React.ReactNode
  header?: string | React.ReactNode
  visible: boolean
  size?: 'medium' | 'large' | 'xlarge' | 'xxlarge'
  loading?: boolean
  align?: 'right' | 'left'
  hideFooter?: boolean
  customFooter?: React.ReactNode | ((props: { loading: boolean }) => React.ReactNode)
  onCancel?: () => void
  cancelText?: string
  onConfirm?: () => void
  confirmText?: string
  triggerElement?: React.ReactNode
}

const SidePanel = ({
  id,
  className,
  children,
  header,
  visible,
  open,
  size = 'medium',
  loading,
  align = 'right',
  hideFooter = false,
  customFooter = undefined,
  onConfirm,
  onCancel,
  confirmText = 'Confirm',
  cancelText = 'Cancel',
  triggerElement,
  defaultOpen,
  ...props
}: SidePanelProps) => {
  const __styles = twSidePanelStyles.sidepanel

  const footerContent = customFooter ? (
    typeof customFooter === 'function' ? (
      customFooter({ loading: loading ?? false })
    ) : (
      customFooter
    )
  ) : (
    <Footer>
      <Button
        variant="primary"
        type="submit"
        disabled={loading}
        loading={loading}
        onClick={() => (onConfirm ? onConfirm() : null)}
      >
        {confirmText}
      </Button>
      <Button disabled={loading} variant="tertiary" onClick={() => (onCancel ? onCancel() : null)}>
        {cancelText}
      </Button>
    </Footer>
  )

  function handleOpenChange(open: boolean) {
    if (visible !== undefined && !open) {
      // controlled component behaviour
      if (onCancel) onCancel()
    } else {
      // un-controlled component behaviour
      // setOpen(open)
    }
  }

  open = open || visible

  return (
    <Dialog.Root open={open} onOpenChange={handleOpenChange} defaultOpen={defaultOpen}>
      {triggerElement && <Dialog.Trigger>{triggerElement}</Dialog.Trigger>}

      <Dialog.Portal>
        <DialogOverylay className={__styles.overlay} />
        <DialogContent
          className={[
            __styles.base,
            __styles.size[size],
            __styles.align[align],
            className && className,
          ].join(' ')}
          onOpenAutoFocus={props.onOpenAutoFocus}
          onCloseAutoFocus={props.onCloseAutoFocus}
          onEscapeKeyDown={props.onEscapeKeyDown}
          onPointerDownOutside={props.onPointerDownOutside}
          onInteractOutside={props.onInteractOutside}
        >
          {header && (
            <Header>
              <HeaderClose variant="tertiary" onClick={() => (onCancel ? onCancel() : null)}>
                <XMarkIcon size={16} />
              </HeaderClose>
              {header}
            </Header>
          )}
          <div className={__styles.contents}>{children}</div>
          {!hideFooter && footerContent}
        </DialogContent>
      </Dialog.Portal>
    </Dialog.Root>
  )
}

export function Separator() {
  const __styles = twSidePanelStyles.sidepanel

  return <div className={__styles.separator}></div>
}

export function Content({
  children,
  className,
}: {
  children: React.ReactNode
  className?: string
}) {
  return <StyledContent className={className}>{children}</StyledContent>
}

export function Section({
  children,
  className,
}: {
  children: React.ReactNode
  className?: string
}) {
  return <StyledSection className={className}>{children}</StyledSection>
}

SidePanel.Section = Section
SidePanel.Content = Content
SidePanel.Separator = Separator
SidePanel.HeaderTitle = HeaderTitle
export default SidePanel
