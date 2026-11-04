declare module '*.mdx' {
  import type { ComponentType } from 'react'

  export const frontmatter: {
    id: string
    title: string
    date: string
    summary?: string
    badge?: 'new' | 'improvement' | 'fix'
  }

  const MDXComponent: ComponentType<{ components?: Record<string, unknown> }>
  export default MDXComponent
}
