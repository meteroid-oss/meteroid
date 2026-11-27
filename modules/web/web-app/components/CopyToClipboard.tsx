import {
  Button,
  ButtonVariants,
  Input,
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from '@ui/components'
import { cn } from '@ui/lib'
import { CheckIcon, CopyIcon } from 'lucide-react'
import { useState } from 'react'
import { toast } from 'sonner'

import { copyToClipboard } from '@/lib/helpers'

/**
 * A value with a copy action: the icon sits quiet until the button is hovered, and a tooltip
 * says what clicking does ("Copy"), then that it did ("Copied").
 */
export const CopyToClipboardButton = ({
  text,
  textToCopy = text,
  buttonClassName,
  className,
  buttonVariant,
}: {
  text: string
  textToCopy?: string
  buttonClassName?: string
  buttonVariant?: ButtonVariants['variant']
  className?: string
}) => {
  const [copied, setCopied] = useState(false)

  return (
    <Tooltip onOpenChange={open => !open && setCopied(false)}>
      <TooltipTrigger asChild>
        <Button
          type="button"
          variant={buttonVariant ?? 'special'}
          size="content"
          hasIcon
          aria-label={`Copy ${textToCopy ?? text}`}
          className={cn(
            'group/copy text-xs px-3 py-2 border-none font-normal transition-colors',
            buttonClassName
          )}
          onClick={ev => {
            ev?.stopPropagation()
            ev?.preventDefault()
            copyToClipboard(textToCopy ?? text, () => {
              setCopied(true)
              toast.success('Copied to clipboard!')
            })
          }}
        >
          <span className={cn('whitespace-nowrap overflow-hidden text-ellipsis', className)}>
            {text}
          </span>
          <CopyIcon
            size="10"
            aria-hidden
            className="shrink-0 text-muted-foreground transition-colors group-hover/copy:text-foreground"
          />
        </Button>
      </TooltipTrigger>
      <TooltipContent side="top">{copied ? 'Copied' : 'Copy'}</TooltipContent>
    </Tooltip>
  )
}

/**
 * A read-only value in a field, with a Copy button closing the field's end: one border and fill,
 * so the button reads as part of it. The value itself can't be focused or selected: the button
 * is the one way to copy it.
 */
export const CopyField = ({
  value,
  label,
  className,
}: {
  value: string
  /** What the value is, for screen readers (the visible label does not point at the field). */
  label: string
  className?: string
}) => {
  const [copied, setCopied] = useState(false)

  return (
    <div className={cn('flex', className)}>
      <Input
        value={value}
        readOnly
        tabIndex={-1}
        aria-label={label}
        className="pointer-events-none select-none rounded-r-none font-mono"
      />
      <button
        type="button"
        aria-label={copied ? 'Copied' : `Copy ${label.toLowerCase()}`}
        onClick={() =>
          copyToClipboard(value, () => {
            setCopied(true)
            toast.success('Copied to clipboard!')
            window.setTimeout(() => setCopied(false), 1500)
          })
        }
        className="flex h-8 shrink-0 items-center gap-1.5 rounded-md rounded-l-none border border-l-0 border-field-border bg-field px-2.5 text-[13px] font-medium text-field-foreground transition-colors hover:bg-foreground/[0.1] focus-visible:outline-none focus-visible:ring-[3px] focus-visible:ring-foreground/8"
      >
        {copied ? (
          <CheckIcon size={14} aria-hidden className="text-success" />
        ) : (
          <CopyIcon size={14} aria-hidden className="text-muted-foreground" />
        )}
        {copied ? 'Copied' : 'Copy'}
      </button>
    </div>
  )
}
