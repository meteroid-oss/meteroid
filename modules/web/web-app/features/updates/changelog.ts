import { editionId } from '@/edition'

import type { ComponentType } from 'react'

export type ChangelogBadge = 'new' | 'improvement' | 'fix'

export interface ChangelogMeta {
  id: string
  title: string
  date: string
  summary?: string
  badge?: ChangelogBadge
  /** Ids of the editions the entry is for; absent, it is for every edition. */
  editions?: string[]
}

export interface ChangelogEntry extends ChangelogMeta {
  /** Compiled MDX body, rendered in the expanded view. */
  Body: ComponentType
  /** Hashed asset URL of the entry's cover image, when it exports one. */
  cover?: string
}

// Entries are authored as MDX and compiled at build time (no markdown runtime
// ships to the client). Bodies are text-light; any images they import are
// emitted as separate hashed assets and only fetched when rendered, so eager
// loading here keeps the JS bundle small while giving us all dates up front for
// the unread count.
const modules = import.meta.glob('../../changelog/*.mdx', { eager: true }) as Record<
  string,
  { frontmatter: ChangelogMeta; default: ComponentType; cover?: string }
>

let entries: ChangelogEntry[] | undefined

export const changelog = (): ChangelogEntry[] =>
  (entries ??= Object.values(modules)
    .filter(({ frontmatter }) => !frontmatter.editions || frontmatter.editions.includes(editionId))
    .map(module => ({ ...module.frontmatter, Body: module.default, cover: module.cover }))
    .sort((a, b) => (a.date < b.date ? 1 : -1)))
