import { Separator, cn } from '@md/ui'
import { ReactNode } from 'react'
import { Link } from 'react-router'

import { ListPageHeader, type Crumb } from '@/components/layouts/ListPageLayout'

/**
 * The frame of a detail page with a sidebar (customer, metric, plan): the list pages' top bar,
 * then the content on the list pages' card surface, from the bar down to the window's bottom
 * edge. Pass `header` for a bar of its own, or `section`, `title` and `actions` for the standard
 * one.
 */
export const DetailPageFrame = ({
  header,
  section,
  title,
  actions,
  children,
}: {
  header?: ReactNode
  section?: Crumb
  title?: ReactNode
  actions?: ReactNode
  children: ReactNode
}) => (
  <div data-full-bleed className="flex h-full flex-col">
    {header ?? (
      <ListPageHeader
        title={title}
        breadcrumbs={section ? [section] : undefined}
        actions={actions}
      />
    )}
    {/* The surface lowers --border to stay visible on it (see `mt-detail-surface`). */}
    <div className="mt-table-card mt-detail-surface mx-3 flex flex-1 flex-col overflow-clip rounded-t-md border-[0.5px] border-b-0 border-foreground/12 bg-card-table">
      {children}
    </div>
  </div>
)

/** The main column and the sidebar, side by side from `lg`, stacked below it. */
export const DetailColumns = ({ children }: { children: ReactNode }) => (
  <div className="flex flex-1 flex-col lg:flex-row">{children}</div>
)

/**
 * The main column: fills the space beside the sidebar, its content centred in it and at most
 * 1170px wide. 48px from the top, 40px at the sides (24px when stacked), 24px at the bottom.
 */
export const DetailMain = ({
  sidebar = true,
  children,
}: {
  /** Off for a page without a sidebar: no rule on the column's side. */
  sidebar?: boolean
  children: ReactNode
}) => (
  <div
    className={cn(
      'min-w-0 flex-1 px-6 pb-6 pt-12 lg:px-10',
      sidebar && 'border-b-[0.5px] border-border lg:border-b-0 lg:border-r-[0.5px]'
    )}
  >
    <div className="mx-auto flex w-full max-w-[1170px] flex-col gap-4">{children}</div>
  </div>
)

/** The sidebar: 380px, against the surface's right edge (full width when stacked). */
export const DetailSidebar = ({ children }: { children: ReactNode }) => (
  <div className="flex w-full flex-col gap-2 lg:w-[380px] lg:shrink-0">{children}</div>
)

/** A 56px tile leading the title: an icon for a catalog item (a customer has its avatar). */
export const DetailIcon = ({ children }: { children: ReactNode }) => (
  <div
    aria-hidden
    className="flex size-14 shrink-0 items-center justify-center rounded-xl border border-border bg-foreground/[0.05] text-foreground/70 [&_svg]:size-6"
  >
    {children}
  </div>
)

/**
 * The item's name over the tabs, as the Home page's greeting, with a quieter line below (an
 * alias, an event code). The lines are tall enough for descenders: `truncate` clips to the line
 * box.
 */
export const DetailTitle = ({
  media,
  title,
  subtitle,
  badge,
}: {
  media?: ReactNode
  title: string
  subtitle?: ReactNode
  /** Beside the name, e.g. a status. */
  badge?: ReactNode
}) => (
  <div className="mb-4 flex min-w-0 items-center gap-4">
    {media}
    <div className="flex min-w-0 flex-col gap-1">
      <div className="flex min-w-0 items-center gap-2">
        <h1 className="truncate font-display text-2xl font-medium leading-[1.25]" title={title}>
          {title}
        </h1>
        {badge}
      </div>
      {subtitle && (
        <span className="truncate text-[13px] leading-[18px] tracking-[-0.13px] text-foreground/60">
          {subtitle}
        </span>
      )}
    </div>
  </div>
)

/**
 * Sections stacked in the main column: 32px above and below the divider between two. Each
 * section draws the divider as its top border, so one can leave it out (`!border-t-0`).
 */
export const DetailSections = ({
  className,
  children,
}: {
  className?: string
  children: ReactNode
}) => (
  <div
    className={cn(
      'flex flex-col [&>*+*]:border-t [&>*+*]:border-border [&>*]:py-8 [&>*:first-child]:pt-0 [&>*:last-child]:pb-0',
      className
    )}
  >
    {children}
  </div>
)

/** A section of the main column: a 16px title (and a line on what it holds), 16px above its content. */
export const DetailSection = ({
  title,
  description,
  actions,
  hidden,
  className,
  children,
}: {
  title: ReactNode
  description?: ReactNode
  /** On the heading's right, e.g. a `SectionAddButton`. */
  actions?: ReactNode
  hidden?: boolean
  className?: string
  children: ReactNode
}) => (
  <section className={cn('flex flex-col gap-4', hidden && 'hidden', className)}>
    <div className="flex min-h-7 items-start justify-between gap-3">
      <div className="flex min-w-0 flex-col gap-1">
        <h3 className="text-base font-medium leading-7">{title}</h3>
        {description && (
          <p className="-mt-1 max-w-2xl text-[13px] leading-snug text-muted-foreground">
            {description}
          </p>
        )}
      </div>
      {actions && <div className="flex shrink-0 items-center gap-2">{actions}</div>}
    </div>
    {children}
  </section>
)

/** A titled group of rows in the sidebar, with an optional action (e.g. Edit) on the right. */
export const SidebarSection = ({
  title,
  action,
  children,
}: {
  title: ReactNode
  action?: ReactNode
  children: ReactNode
}) => (
  <div className="flex flex-col gap-3 p-6">
    <div className="mb-2 flex h-7 items-center justify-between gap-2">
      <div className="text-[15px] font-medium">{title}</div>
      {action}
    </div>
    {children}
  </div>
)

/** The 0.5px rule between two sidebar sections. */
export const SidebarSeparator = () => <Separator className="-my-3 h-[0.5px]" />

/**
 * A label and its value on one line in the sidebar. With `link` the value leads to a page in the
 * app, with `externalLink` it links out.
 */
export const DetailRow = ({
  title,
  value,
  link,
  externalLink,
}: {
  title: string
  value?: string | ReactNode
  link?: string
  externalLink?: string
}) => (
  <div className="flex items-center justify-between gap-4">
    <div className="shrink-0 text-[13px] text-muted-foreground">{title}</div>
    <div className="min-w-0 max-w-[250px] break-words text-right">
      {externalLink ? (
        <a href={externalLink} target="_blank" rel="noopener noreferrer">
          <div className="text-[13px] text-brand hover:underline">{value ?? 'N/A'}</div>
        </a>
      ) : link ? (
        <Link to={link} className="text-[13px] text-brand hover:underline">
          {value}
        </Link>
      ) : (
        <div className="text-[13px]">{value ?? '-'}</div>
      )}
    </div>
  </div>
)
