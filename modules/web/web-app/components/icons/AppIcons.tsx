import { cn } from '@md/ui'

import AddSvg from '@/assets/icons/add.svg?react'
import BillingSvg from '@/assets/icons/billing.svg?react'
import CatalogSvg from '@/assets/icons/catalog.svg?react'
import ChevronDownSvg from '@/assets/icons/chevron-down.svg?react'
import ChevronsUpDownSvg from '@/assets/icons/chevrons-up-down.svg?react'
import EventsSvg from '@/assets/icons/events.svg?react'
import FilterSvg from '@/assets/icons/filter.svg?react'
import HomeSvg from '@/assets/icons/home.svg?react'
import OrgAvatarSvg from '@/assets/icons/org-avatar.svg?react'
import PanelSvg from '@/assets/icons/panel.svg?react'
import PillPlusSvg from '@/assets/icons/pill-plus.svg?react'
import PlusSvg from '@/assets/icons/plus.svg?react'
import SearchSvg from '@/assets/icons/search.svg?react'
import SelectorChevronsSvg from '@/assets/icons/selector-chevrons.svg?react'
import SettingsSvg from '@/assets/icons/settings.svg?react'
import SupportSvg from '@/assets/icons/support.svg?react'
import UsersSvg from '@/assets/icons/users.svg?react'
import ViewsSvg from '@/assets/icons/views.svg?react'

import type { ComponentType, SVGProps } from 'react'

export type AppIcon = ComponentType<SVGProps<SVGSVGElement>>

/**
 * The design's icons ship with fixed stroke/fill colours. Painting them with `currentColor` lets
 * the surrounding text colour drive them, so hover, active and button states recolour the icon.
 */
const TINT = '[&_[stroke]]:stroke-current [&_path[fill]:not([fill=none])]:fill-current'

const tinted =
  (Svg: AppIcon): AppIcon =>
  ({ className, ...props }) => (
    <Svg aria-hidden className={cn('shrink-0', TINT, className)} {...props} />
  )

/** A nav icon with its filled look for the selected item (see NavMain). */
export type FillableIcon = AppIcon & { filledClassName?: string }

/**
 * Fills the outline icon for the selected nav item. There are no filled drawings, so the shapes
 * are painted with `currentColor`; inner detail lines (a hexagon's edges, a page's fold) are drawn
 * in the selected item's background so they stay readable on the fill.
 */
const fillable = (Svg: AppIcon, filledClassName: string): FillableIcon =>
  Object.assign(tinted(Svg), { filledClassName })

const FILL_ALL = '[&_path]:fill-current'

export const HomeIcon = fillable(HomeSvg, FILL_ALL)
export const CustomersIcon = fillable(UsersSvg, FILL_ALL)
// The hexagon is the first path; the edges meeting in the middle follow.
export const CatalogIcon = fillable(
  CatalogSvg,
  '[&_path:first-of-type]:fill-current [&_path:not(:first-of-type)]:stroke-sidebar-accent'
)
// The paths: the outline's page fold, the page, and the filled look's fold (hidden in the outline).
// Filled, the fold is a square corner cut through the page, so the folded flap reads on the fill.
export const BillingIcon = fillable(
  BillingSvg,
  '[&_path:nth-of-type(2)]:fill-current [&_path:first-of-type]:hidden [&_path:last-of-type]:inline [&_path:last-of-type]:stroke-sidebar-accent'
)
export const EventsIcon = fillable(EventsSvg, FILL_ALL)
export const NewIcon = tinted(PlusSvg)
export const ChevronDownIcon = tinted(ChevronDownSvg)
export const ChevronsUpDownIcon = tinted(ChevronsUpDownSvg)
export const SidebarPanelIcon = tinted(PanelSvg)
export const SettingsIcon = tinted(SettingsSvg)
export const SupportIcon = tinted(SupportSvg)
export const FilterIcon = tinted(FilterSvg)
export const SearchIcon = tinted(SearchSvg)
export const ViewsIcon = tinted(ViewsSvg)
/** The up/down chevrons on a pill selector (the tenant selector). */
export const SelectorChevronsIcon = tinted(SelectorChevronsSvg)
/** The "+" on the Home quick-create pills; coloured by the pill. */
export const PillPlusIcon = tinted(PillPlusSvg)

/** The "+" on create buttons ("New plan"), drawn in its own grey on the button. */
export const AddIcon = ({ className, ...props }: SVGProps<SVGSVGElement>) => (
  <AddSvg aria-hidden className={cn('shrink-0', className)} {...props} />
)

/** Default workspace mark (the workspace has no uploaded logo): the filled org avatar, which
 *  carries its own dark tile. */
export const WorkspaceMark = ({ className }: { className?: string }) => (
  <span
    aria-hidden
    className={cn(
      'flex size-5 shrink-0 items-center justify-center overflow-hidden rounded-[5px] border-[0.5px] border-white/8',
      className
    )}
  >
    <OrgAvatarSvg className="size-full shrink-0" />
  </span>
)

/** One-letter avatar, e.g. the "C" before a customer name or the user's "P" in the sidebar. */
export const InitialAvatar = ({
  name,
  shape = 'square',
  className,
}: {
  name?: string
  shape?: 'square' | 'circle'
  className?: string
}) => (
  <span
    aria-hidden
    data-avatar
    className={cn(
      'inline-flex shrink-0 items-center justify-center font-medium uppercase leading-none',
      shape === 'square'
        ? // Dark: background two steps above the accent (#2e2e2e) and the letter two steps above the
          // faint text (#737373), so both stand off the card.
          'size-4 rounded-[3px] bg-accent text-[9px] text-faint-foreground dark:bg-[hsl(0_0%_23.5%)] dark:text-[hsl(0_0%_56%)]'
        : 'size-5 rounded-full bg-[#00809e] text-[11px] text-white',
      className
    )}
  >
    {name?.trim().charAt(0) || '?'}
  </span>
)
