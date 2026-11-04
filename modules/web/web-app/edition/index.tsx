import edition from 'virtual:edition'

import type { ComponentSlots, NavMainSection, Slots, WebModule } from '@/edition/types'
import type { ComponentType } from 'react'
import type { RouteObject } from 'react-router-dom'

// Read the edition when called or rendered, never at module load: the edition's modules import
// core, so a core module can load before the edition is initialised.
export const editionId = edition.id

export const useModule = (id: string): WebModule | undefined =>
  edition.modules.find(module => module.id === id)

// Typed through `unknown`: a generic key into the slot map builds a union too large for the checker.
export const slotItems = <K extends keyof Slots>(name: K): Slots[K][] => {
  const items: unknown[] = []
  for (const module of edition.modules) {
    const slots = module.slots as Partial<Record<keyof Slots, unknown[]>> | undefined
    items.push(...(slots?.[name] ?? []))
  }
  return items as Slots[K][]
}

export const hasSlot = (name: keyof Slots): boolean => slotItems(name).length > 0

/**
 * Core entries with module items placed after the entry their `after` names; the other items
 * take position `fallback` (default: the end).
 */
export const placeAfter = <T extends { after?: string }>(
  entries: T[],
  items: T[],
  key: (entry: T) => string,
  fallback = entries.length
): T[] => {
  const anchors = new Set(entries.map(key))
  const loose = items.filter(item => item.after === undefined || !anchors.has(item.after))
  return [
    ...entries.flatMap((entry, index) => [
      ...(index === fallback ? loose : []),
      entry,
      ...items.filter(item => item.after === key(entry)),
    ]),
    ...(fallback >= entries.length ? loose : []),
  ]
}

export const editionRootRoutes = (): RouteObject[] =>
  edition.modules.flatMap(module => module.rootRoutes ?? [])

export const editionProtectedRoutes = (): RouteObject[] =>
  edition.modules.flatMap(module => module.protectedRoutes ?? [])

export const editionOrgRoutes = (): RouteObject[] =>
  edition.modules.flatMap(module => module.orgRoutes ?? [])

export const editionTenantRoutes = (): RouteObject[] =>
  edition.modules.flatMap(module => module.tenantRoutes ?? [])

export const editionPortalRoutes = (): RouteObject[] =>
  edition.modules.flatMap(module => module.portalRoutes ?? [])

// The module list is fixed at build time, so each render calls the same hooks in the same order.
export const useVisibleItems = <T extends { useVisible: (ctx: C) => boolean }, C>(
  items: T[],
  ctx: C
): T[] => items.filter(item => item.useVisible(ctx))

/** The modules' main navigation sections, each with what waits on the caller there. */
export const useNavMainSections = (): (NavMainSection & { counts: Record<string, number> })[] =>
  slotItems('layout.navMain').map(section => ({ ...section, counts: section.useCounts?.() ?? {} }))

/** Renders every component contributed to a component slot. */
export const Slot = <K extends keyof ComponentSlots>({
  name,
  ...props
}: { name: K } & ComponentSlots[K]) => (
  <>
    {(slotItems(name) as unknown as ComponentType<typeof props>[]).map((Component, index) => (
      <Component key={index} {...props} />
    ))}
  </>
)
