import { Card, cn } from '@md/ui'

import type { QuoteSectionId } from '@/features/quotes/editor/editorSections'
import type { ReactNode } from 'react'

interface EditorSectionProps {
  id: QuoteSectionId
  title: ReactNode
  /** Right of the title: a switch or a select bound to the section. */
  aside?: ReactNode
  className?: string
  children: ReactNode
}

export const EditorSection = ({ id, title, aside, className, children }: EditorSectionProps) => (
  <section id={id} aria-labelledby={`${id}-title`} className="scroll-mt-28">
    <Card className={cn('flex flex-col gap-4 p-5', className)}>
      <div className="flex flex-wrap items-start justify-between gap-3">
        <h2 id={`${id}-title`} className="text-base font-semibold">
          {title}
        </h2>
        {aside}
      </div>
      {children}
    </Card>
  </section>
)
