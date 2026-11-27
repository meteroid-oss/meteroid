import { cn } from '@ui/lib'

import { CopyToClipboardButton } from '@/components/CopyToClipboard'

export const LocalId = ({
  localId,
  buttonClassName,
  className,
}: {
  localId: string
  buttonClassName?: string
  className?: string
}) => {
  return (
    <CopyToClipboardButton
      text={localId}
      // White at 7.2% over whatever it sits on: visible on the table card as on the page.
      // (The button's own variant sets a dark-mode fill, so dark is named too.)
      buttonClassName={cn(
        'bg-foreground/[0.072] text-secondary-foreground hover:bg-foreground/[0.1] dark:bg-foreground/[0.072] dark:hover:bg-foreground/[0.1]',
        buttonClassName
      )}
      // An identifier: mono, in capitals (shown so; the copy keeps it as it is).
      className={cn('font-mono uppercase tracking-wide', className)}
    />
  )
}
