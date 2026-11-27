import { Button, type ButtonProps, cn } from '@md/ui'
import { ArrowLeft, Check, X } from 'lucide-react'
import { forwardRef, type ReactNode } from 'react'

import { useForceTheme } from 'providers/ThemeProvider'

/**
 * A full-screen, step-by-step flow (the go-live "Activate your account" flow): a 56px top bar
 * with a close button and the flow's title, and a centred 460px column below it.
 *
 * Purely presentational: the page keeps its own form, fields, validation and mutations, and
 * wraps them in these pieces. Fields inside keep the app's own field design.
 */
export const WizardLayout = ({
  title,
  onClose,
  children,
}: {
  title: string
  onClose: () => void
  children: ReactNode
}) => {
  useForceTheme('dark')

  return (
    <div className="dark min-h-screen bg-background text-foreground">
      <header className="sticky top-0 z-10 flex h-14 items-center gap-3 border-b border-white/10 bg-background pl-5 pr-3">
        <button
          type="button"
          aria-label="Close"
          onClick={onClose}
          className="-m-1 grid size-6 place-items-center rounded-[4px] text-[#afafaf] transition-colors hover:bg-white/8 hover:text-[#ebebeb] focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
        >
          <X size={14} strokeWidth={1.6} />
        </button>
        <div className="h-4 w-px bg-white/10" aria-hidden />
        <h1 className="text-[13px] font-medium leading-none text-[#ebebeb]">{title}</h1>
      </header>
      <main className="flex flex-col items-center px-6 pb-16 pt-16">{children}</main>
    </div>
  )
}

/** The 460px column the stepper, step header, fields and actions share. */
export const WizardColumn = ({
  className,
  children,
}: {
  className?: string
  children: ReactNode
}) => <div className={cn('w-full max-w-[460px]', className)}>{children}</div>

/**
 * The steps as a pill: one 56×32 segment per step. The current step is raised, a done step is
 * green with a check, a step ahead stays flat.
 */
export const WizardStepper = ({
  steps,
  current,
  className,
}: {
  steps: number
  /** Zero-based. */
  current: number
  className?: string
}) => (
  <ol
    aria-label="Progress"
    className={cn('flex w-fit items-center gap-2 rounded-full bg-[#272727] p-0.5', className)}
  >
    {Array.from({ length: steps }, (_, index) => {
      const state = index < current ? 'done' : index === current ? 'current' : 'upcoming'
      return (
        <li
          key={index}
          aria-current={state === 'current' ? 'step' : undefined}
          className={cn(
            'flex h-8 w-14 items-center justify-center rounded-full text-[13px] font-medium leading-5 text-[#dcdcdc]',
            state === 'current' && 'bg-[#1a1a1a]',
            state === 'done' && 'bg-[#a1ee77] text-[#263223]'
          )}
        >
          {state === 'done' ? (
            <>
              <Check size={14} strokeWidth={1.6} aria-hidden />
              <span className="sr-only">Step {index + 1}, done</span>
            </>
          ) : (
            <>
              <span className="sr-only">Step </span>
              {index + 1}
            </>
          )}
        </li>
      )
    })}
  </ol>
)

/** The step's title and what it is for. */
export const WizardStepHeader = ({
  title,
  description,
  className,
}: {
  title: ReactNode
  description?: ReactNode
  className?: string
}) => (
  <div className={cn('flex flex-col gap-2 pb-6 pt-8', className)}>
    <h2 className="text-lg font-medium leading-5 text-[#f2f2f2]">{title}</h2>
    {description && (
      <p className="text-[13px] leading-[1.45] tracking-[-0.13px] text-[#f1f0ef]/60">
        {description}
      </p>
    )}
  </div>
)

/** A group of fields under one label, such as the lines of an address. */
export const WizardFieldGroup = ({ label, children }: { label: string; children: ReactNode }) => (
  <fieldset>
    {/* Drawn as a form field's label (GenericFormField), the same distance above its fields. */}
    <legend className="mb-3 text-xs leading-5 text-foreground/60 dark:text-muted-foreground">
      {label}
    </legend>
    <div className="flex flex-col gap-4">{children}</div>
  </fieldset>
)

/** The step's actions, below a divider: an optional Back, then the main action filling the row. */
export const WizardActions = ({
  className,
  children,
}: {
  className?: string
  children: ReactNode
}) => (
  <div className={cn('flex flex-col gap-6', className)}>
    <div className="h-px bg-white/10" aria-hidden />
    <div className="flex items-center gap-4">{children}</div>
  </div>
)

export const WizardBackButton = forwardRef<HTMLButtonElement, ButtonProps>(
  ({ className, children = 'Back', ...props }, ref) => (
    <Button
      ref={ref}
      type="button"
      variant="ghost"
      size="content"
      className={cn(
        'h-8 gap-2 rounded-md bg-white/12 px-3 tracking-[-0.13px] text-[#f1f0ef] hover:bg-white/16 hover:text-[#f1f0ef]',
        className
      )}
      {...props}
    >
      <ArrowLeft size={12} strokeWidth={1.6} className="text-[#9b9a9a]" aria-hidden />
      {children}
    </Button>
  )
)
WizardBackButton.displayName = 'WizardBackButton'

export const WizardPrimaryButton = forwardRef<HTMLButtonElement, ButtonProps>(
  ({ className, ...props }, ref) => (
    <Button
      ref={ref}
      variant="primary"
      size="content"
      className={cn(
        'h-8 flex-1 gap-2 rounded-md bg-[#f1f0ef] px-3 tracking-[-0.13px] text-[#0b0b0b] hover:bg-[#f1f0ef]/90',
        'shadow-[0_2px_2px_-1px_rgb(0_0_0/0.12),0_1px_1px_-1px_rgb(0_0_0/0.12),0_0_0_0.5px_rgb(0_0_0/0.4),inset_0_-6px_12px_-4px_rgb(0_0_0/0.16),inset_0_0_1px_1px_rgb(255_255_255/0.16)]',
        className
      )}
      {...props}
    />
  )
)
WizardPrimaryButton.displayName = 'WizardPrimaryButton'
