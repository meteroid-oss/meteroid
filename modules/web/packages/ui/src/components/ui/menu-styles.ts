// Shared look of every floating menu (dropdown menus, selects, popovers and the command lists in
// them), from the Figma dropdown: a solid grey surface with a hairline border, and 30px items with
// subtle 13px text that turns white on the hovered or keyboard-focused item.

/** The floating surface: a 0.5px border at 12% and soft shadow layers that fade out deep. Pair
 *  with the component's own padding. Any divider inside (a separator, or a line drawn as
 *  `border-t` / `border-b` by the content) is drawn at 12% too. */
export const menuSurface =
  'rounded-md border-[0.5px] border-foreground/12 bg-menu text-menu-foreground [&_[role=separator]]:bg-foreground/12 [&_[role=none][data-orientation]]:bg-foreground/12 [&_.border-b]:border-foreground/12 [&_.border-t]:border-foreground/12 shadow-[0_62px_62px_3px_rgba(0,0,0,0.05),0_32px_32px_rgba(0,0,0,0.1),0_16px_16px_rgba(0,0,0,0.1),0_8px_8px_rgba(0,0,0,0.05),0_2px_2px_rgba(0,0,0,0.05),0_1px_1px_rgba(0,0,0,0.05)]'

/**
 * One row; `min-h` so two-line items still fit. The text is the menu's full foreground; leading
 * icons sit a step quieter (any svg without its own colour), so the label leads. The highlight is
 * a `::before` layer behind the text that fades in while growing from 97% to full size, and gives
 * a little (98.5%) while pressed, as the nav's items do. Each
 * component turns it on under its own state variant (Tailwind only sees classes written out in
 * full):
 *   `<state>:before:opacity-100 <state>:before:scale-100`
 */
export const menuItem =
  "relative isolate min-h-[30px] cursor-pointer gap-2 rounded-md py-1.5 text-[13px] leading-[18px] font-medium text-menu-foreground transition-colors duration-150 before:absolute before:inset-0 before:-z-10 before:rounded-[inherit] before:bg-menu-accent before:opacity-0 before:scale-[0.97] before:transition-[opacity,scale] before:duration-150 motion-safe:active:before:!scale-[0.985] motion-reduce:before:scale-100 [&_svg]:shrink-0 [&_svg:not([class*='size-']):not([class*='h-'])]:size-4 [&_svg:not([class*='text-'])]:text-menu-foreground/60"

/** A group's title: small, quieter than the items, so it reads as a heading over them. */
export const menuLabel = 'px-2 pt-2 pb-1 text-xs font-medium text-menu-foreground/55'

/** Trailing text in a row (a shortcut, a count, a hint): quiet, pushed to the end. */
export const menuTrailing = 'ml-auto pl-3 text-xs font-normal text-menu-foreground/50'

/** An item that removes or ends something. */
export const menuItemDestructive =
  "text-destructive before:bg-destructive/15 [&_svg:not([class*='text-'])]:text-destructive"

/** Divider between groups, edge to edge. */
export const menuSeparator = '-mx-1 my-1 h-px bg-foreground/12'
