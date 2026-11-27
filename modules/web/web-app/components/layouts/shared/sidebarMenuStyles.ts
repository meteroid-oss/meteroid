// The sidebar's own dropdowns (organisation and tenant), from the Figma flows: a dark panel with
// 10px corners (border and shadow are every menu's, from menu-styles). Items are 28px with
// grey text; the current or hovered item gets a light fill and full-contrast text. These classes
// are layered over the shared menu look (menu-styles in @md/ui), so the hover animation stays.

export const SIDEBAR_MENU_CONTENT = 'flex flex-col gap-0.5 rounded-[10px] bg-card p-0.5'

export const SIDEBAR_MENU_ITEM =
  'min-h-7 gap-2.5 px-2 py-1.5 text-[13px] leading-[18px] tracking-[-0.15px] text-foreground/60 before:bg-foreground/[0.05] focus:text-foreground'

/** The item for the organisation / tenant you are in. */
export const SIDEBAR_MENU_ITEM_CURRENT = 'text-foreground before:opacity-100 before:scale-100'

export const SIDEBAR_MENU_SEPARATOR = '-mx-0.5 my-0 bg-foreground/12'
