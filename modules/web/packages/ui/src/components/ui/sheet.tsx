import * as SheetPrimitive from '@radix-ui/react-dialog'
import { Cross2Icon } from '@radix-ui/react-icons'
import { cva, type VariantProps } from 'class-variance-authority'
import * as React from 'react'

import { Button } from '@ui/components/ui/button'
import { cn } from '@ui/lib'

const Sheet = SheetPrimitive.Root

const SheetTrigger = SheetPrimitive.Trigger

const SheetClose = SheetPrimitive.Close

const SheetPortal = SheetPrimitive.Portal

const SheetOverlay = React.forwardRef<
  React.ElementRef<typeof SheetPrimitive.Overlay>,
  React.ComponentPropsWithoutRef<typeof SheetPrimitive.Overlay>
>(({ className, ...props }, ref) => (
  <SheetPrimitive.Overlay
    className={cn('mt-backdrop fixed inset-0 z-50', className)}
    {...props}
    ref={ref}
  />
))
SheetOverlay.displayName = SheetPrimitive.Overlay.displayName

// Only the right-hand panel uses the shared side-panel motion (`mt-sheet`); the other sides keep
// their full-width slide.
const edgeSlide =
  'transition ease-in-out data-[state=open]:animate-in data-[state=closed]:animate-out data-[state=closed]:duration-300 data-[state=open]:duration-500'

const sheetVariants = cva(
  'fixed z-50 gap-4 bg-background dark:bg-[#171717] p-6 shadow-lg overflow-y-auto',
  {
    variants: {
      side: {
        top: `${edgeSlide} inset-x-0 top-0 border-b data-[state=closed]:slide-out-to-top-[100%] data-[state=open]:slide-in-from-top-[100%]`,
        bottom: `${edgeSlide} inset-x-0 bottom-0 border-t data-[state=closed]:slide-out-to-bottom-[100%] data-[state=open]:slide-in-from-bottom-[100%]`,
        left: `${edgeSlide} inset-y-0 left-0 h-full w-3/4 border-r data-[state=closed]:slide-out-to-left-[100%] data-[state=open]:slide-in-from-left-[100%]`,
        // Floats 8px off the window edge as a rounded panel with a 0.5px border at 12%.
        right:
          'mt-sheet flex flex-col inset-y-0 right-0 h-full w-3/4 border-l sm:inset-y-2 sm:right-2 sm:h-auto sm:rounded-[10px] sm:border-[0.5px] sm:border-foreground/12 sm:shadow-[0_0_0_1px_rgba(0,0,0,0.6),0_4px_6px_-4px_rgba(16,24,40,0.1),0_10px_15px_-3px_rgba(0,0,0,0.1)]',
      },
      size: {
        vertical: '',
        small: 'w-full sm:min-w-[450px] sm:w-[450px] ',
        medium: 'w-full sm:min-w-[580px] sm:w-[580px]',
        large: 'w-full sm:w-2/3 lg:w-1/2 sm:min-w-[580px]',
        // Near-full-screen for split-pane forms (form + live preview).
        xlarge: 'w-full sm:w-[96vw] sm:max-w-[1700px] sm:min-w-[580px]',
      },
    },
    defaultVariants: {
      side: 'right',
      size: 'large',
    },
  }
)

interface SheetContentProps
  extends
    React.ComponentPropsWithoutRef<typeof SheetPrimitive.Content>,
    VariantProps<typeof sheetVariants> {}

const SheetContent = React.forwardRef<
  React.ElementRef<typeof SheetPrimitive.Content>,
  SheetContentProps
>(({ side = 'right', size, className, children, ...props }, ref) => (
  <SheetPortal>
    <SheetOverlay
      className={
        side === 'right'
          ? 'mt-sheet-overlay'
          : 'data-[state=open]:animate-in data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0'
      }
    />
    <SheetPrimitive.Content
      ref={ref}
      className={cn(sheetVariants({ side, size }), className)}
      {...props}
    >
      {children}
      {/* A 26px round close button: a faint fill, the cross in the quiet grey. */}
      <SheetPrimitive.Close className="absolute right-4 top-4 rounded-full ring-offset-background focus:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 disabled:pointer-events-none">
        <Button
          size="icon"
          variant="ghost"
          className="size-[26px] rounded-full bg-foreground/[0.04] text-faint-foreground hover:bg-foreground/10 hover:text-foreground"
        >
          <Cross2Icon className="size-3.5" />
        </Button>
        <span className="sr-only">Close</span>
      </SheetPrimitive.Close>
    </SheetPrimitive.Content>
  </SheetPortal>
))
SheetContent.displayName = SheetPrimitive.Content.displayName

const SheetHeader = ({ className, ...props }: React.HTMLAttributes<HTMLDivElement>) => (
  <div className={cn('flex flex-col space-y-2 text-center sm:text-left', className)} {...props} />
)
SheetHeader.displayName = 'SheetHeader'

const SheetFooter = ({ className, ...props }: React.HTMLAttributes<HTMLDivElement>) => (
  <div
    className={cn('flex flex-col-reverse gap-2.5 sm:flex-row sm:justify-end', className)}
    {...props}
  />
)
SheetFooter.displayName = 'SheetFooter'

const SheetTitle = React.forwardRef<
  React.ElementRef<typeof SheetPrimitive.Title>,
  React.ComponentPropsWithoutRef<typeof SheetPrimitive.Title>
>(({ className, ...props }, ref) => (
  <SheetPrimitive.Title
    ref={ref}
    className={cn('font-display text-lg leading-[30px] font-medium text-foreground', className)}
    {...props}
  />
))
SheetTitle.displayName = SheetPrimitive.Title.displayName

const SheetDescription = React.forwardRef<
  React.ElementRef<typeof SheetPrimitive.Description>,
  React.ComponentPropsWithoutRef<typeof SheetPrimitive.Description>
>(({ className, ...props }, ref) => (
  <SheetPrimitive.Description
    ref={ref}
    className={cn('text-sm text-muted-foreground', className)}
    {...props}
  />
))
SheetDescription.displayName = SheetPrimitive.Description.displayName

export {
  Sheet,
  SheetClose,
  SheetContent,
  SheetDescription,
  SheetFooter,
  SheetHeader,
  SheetOverlay,
  SheetPortal,
  SheetTitle,
  SheetTrigger,
}
