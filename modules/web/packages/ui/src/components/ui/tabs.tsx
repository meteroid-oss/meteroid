import * as TabsPrimitive from '@radix-ui/react-tabs'
import { cva, type VariantProps } from 'class-variance-authority'
import * as React from 'react'

import { cn } from '@ui/lib'

const Tabs = TabsPrimitive.Root

const tabsListVariants = cva('', {
  variants: {
    variant: {
      // Contained: a rounded bar with a pill that slides to the active tab (`mt-tabs` in main.css).
      default: 'mt-tabs',
      ghost: 'inline-flex items-center text-muted-foreground h-[30px] gap-0 bg-transparent p-0.5',
      // Underlined: a line under the active tab that slides the same way.
      underline:
        'relative inline-flex h-auto items-center justify-start gap-6 rounded-none border-b border-border bg-transparent p-0',
    },
  },
  defaultVariants: {
    variant: 'default',
  },
})

const tabsTriggerVariants = cva(
  'inline-flex items-center justify-center whitespace-nowrap font-medium disabled:pointer-events-none disabled:opacity-50',
  {
    variants: {
      variant: {
        default: 'mt-tab text-sm',
        ghost:
          'ring-offset-background transition-all focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 h-full rounded-md px-3 text-[13px] leading-4 text-faint-foreground hover:text-foreground data-[state=active]:bg-accent data-[state=active]:text-card-foreground',
        underline:
          'relative z-[1] ring-offset-background transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 rounded-none border-b-2 border-transparent px-0 pb-2.5 text-sm text-muted-foreground hover:text-foreground data-[state=active]:text-foreground',
      },
    },
    defaultVariants: {
      variant: 'default',
    },
  }
)

type TabsVariant = NonNullable<VariantProps<typeof tabsListVariants>['variant']>

const TabsVariantContext = React.createContext<TabsVariant>('default')

/**
 * Keeps the indicator (pill or line) under the active tab. A change of tab slides it there; the
 * first placement and any change in size (fonts loading, a narrower window, a panel that was
 * closed and measured 0) put it in place at once.
 */
function useActiveTabIndicator(enabled: boolean) {
  const listRef = React.useRef<HTMLDivElement | null>(null)
  const indicatorRef = React.useRef<HTMLSpanElement | null>(null)

  React.useLayoutEffect(() => {
    const list = listRef.current
    const indicator = indicatorRef.current
    if (!enabled || !list || !indicator) return

    const activeTab = () =>
      Array.from(list.querySelectorAll<HTMLElement>('[role="tab"][data-state="active"]')).find(
        tab => tab.closest('[role="tablist"]') === list
      )

    const place = (animate: boolean) => {
      const tab = activeTab()
      if (!tab) {
        indicator.style.opacity = '0'
        return
      }
      if (!animate) indicator.style.transition = 'none'
      indicator.style.opacity = ''
      indicator.style.setProperty('--tabs-pill-x', `${tab.offsetLeft}px`)
      indicator.style.setProperty('--tabs-pill-w', `${tab.offsetWidth}px`)
      if (!animate) {
        void indicator.offsetWidth
        indicator.style.transition = ''
      }
    }

    // Sizes: the list and every tab in it. Observing fires once right away: the first placement.
    const resize = new ResizeObserver(() => place(false))
    const observeTabs = () => {
      resize.disconnect()
      resize.observe(list)
      list.querySelectorAll('[role="tab"]').forEach(tab => resize.observe(tab))
    }
    observeTabs()

    // The active tab changing (click, arrow keys, or a controlled value) is the one move that slides.
    const mutations = new MutationObserver(records => {
      if (records.some(r => r.type === 'childList')) {
        observeTabs()
        place(false)
      } else {
        place(true)
      }
    })
    mutations.observe(list, {
      subtree: true,
      childList: true,
      attributes: true,
      attributeFilter: ['data-state'],
    })

    return () => {
      resize.disconnect()
      mutations.disconnect()
    }
  }, [enabled])

  return { listRef, indicatorRef }
}

const TabsList = React.forwardRef<
  React.ElementRef<typeof TabsPrimitive.List>,
  React.ComponentPropsWithoutRef<typeof TabsPrimitive.List> & VariantProps<typeof tabsListVariants>
>(({ className, variant, children, ...props }, ref) => {
  const resolved = variant ?? 'default'
  const indicator =
    resolved === 'default' ? 'mt-tabs-pill' : resolved === 'underline' ? 'mt-tabs-line' : null
  const { listRef, indicatorRef } = useActiveTabIndicator(indicator !== null)

  const setRefs = React.useCallback(
    (node: HTMLDivElement | null) => {
      listRef.current = node
      if (typeof ref === 'function') ref(node)
      else if (ref) ref.current = node
    },
    [ref, listRef]
  )

  return (
    <TabsVariantContext.Provider value={resolved}>
      <TabsPrimitive.List
        ref={setRefs}
        className={cn(tabsListVariants({ variant }), className)}
        {...props}
      >
        {indicator && <span ref={indicatorRef} aria-hidden="true" className={indicator} />}
        {children}
      </TabsPrimitive.List>
    </TabsVariantContext.Provider>
  )
})
TabsList.displayName = TabsPrimitive.List.displayName

const TabsTrigger = React.forwardRef<
  React.ElementRef<typeof TabsPrimitive.Trigger>,
  React.ComponentPropsWithoutRef<typeof TabsPrimitive.Trigger> &
    VariantProps<typeof tabsTriggerVariants>
>(({ className, variant, ...props }, ref) => {
  const contextVariant = React.useContext(TabsVariantContext)
  return (
    <TabsPrimitive.Trigger
      ref={ref}
      className={cn(tabsTriggerVariants({ variant: variant ?? contextVariant }), className)}
      {...props}
    />
  )
})
TabsTrigger.displayName = TabsPrimitive.Trigger.displayName

const TabsContent = React.forwardRef<
  React.ElementRef<typeof TabsPrimitive.Content>,
  React.ComponentPropsWithoutRef<typeof TabsPrimitive.Content>
>(({ className, ...props }, ref) => (
  <TabsPrimitive.Content
    ref={ref}
    className={cn(
      'mt-2 ring-offset-background focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2',
      className
    )}
    {...props}
  />
))
TabsContent.displayName = TabsPrimitive.Content.displayName

export { Tabs, TabsList, TabsTrigger, TabsContent }
