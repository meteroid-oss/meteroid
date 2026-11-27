import { ReactNode } from 'react'

/**
 * One fact of a `FactStrip`: a cell at least 180px wide, a 12px label over a 13px value. The
 * cells draw their dividers on their left and top edges; the strip hides those on its own outer
 * edge, so the lines stay whole however the cells wrap.
 */
export const Fact = ({ label, children }: { label: ReactNode; children: ReactNode }) => (
  <div className="flex min-w-[180px] flex-1 basis-[180px] flex-col gap-1.5 border-l border-t border-border px-4 py-3">
    <dt className="text-xs leading-none text-foreground/60">{label}</dt>
    <dd className="min-w-0 text-[13px] leading-5 text-foreground">{children}</dd>
  </div>
)

/**
 * A detail page's key facts in one raised strip rather than a card each (a subscription's period
 * and payment, a plan version's settings). Cells wrap onto more rows on narrow screens.
 */
export const FactStrip = ({ label, children }: { label: string; children: ReactNode }) => (
  <div className="min-w-0 flex-1 overflow-hidden rounded-lg border border-border bg-card dark:bg-foreground/[0.035]">
    {/* Pulled 1px up and left, so the cells' dividers on the strip's own edge fall outside it. */}
    <dl aria-label={label} className="-ml-px -mt-px flex flex-wrap">
      {children}
    </dl>
  </div>
)
