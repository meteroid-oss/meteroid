import { cn } from '@md/ui'

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

/** One block of the quote form: a flat band with an 18px title, divided from the next by a line. */
export const EditorSection = ({ id, title, aside, className, children }: EditorSectionProps) => (
  <section
    id={id}
    aria-labelledby={`${id}-title`}
    className={cn(
      'flex scroll-mt-[4.5rem] flex-col gap-5 border-b border-border py-8 first:pt-10 last:border-b-0',
      className
    )}
  >
    <div className="flex flex-wrap items-center justify-between gap-3">
      <h2 id={`${id}-title`} className="text-[18px] font-medium leading-5 text-foreground">
        {title}
      </h2>
      {aside}
    </div>
    {children}
  </section>
)
