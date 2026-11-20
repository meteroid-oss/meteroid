import { cn } from '@md/ui'
import { Check, CircleAlert } from 'lucide-react'
import { useEffect, useState } from 'react'
import { useFormContext, useWatch } from 'react-hook-form'

import {
  DEAL_FIELDS,
  QUOTE_SECTIONS,
  TEMPLATE_SECTIONS,
  formIssues,
} from '@/features/quotes/editor/editorSections'

import type {
  QuoteSectionId,
  QuoteSectionLink,
  SectionIssue,
} from '@/features/quotes/editor/editorSections'
import type { QuoteFormValues } from '@/features/quotes/editor/quoteFormSchema'

export const scrollToSection = (id: QuoteSectionId) =>
  document.getElementById(id)?.scrollIntoView({ behavior: 'smooth', block: 'start' })

const useActiveSection = (sections: readonly QuoteSectionLink[]) => {
  const [active, setActive] = useState<QuoteSectionId>(sections[0].id)

  useEffect(() => {
    const observer = new IntersectionObserver(
      entries => {
        const visible = entries
          .filter(e => e.isIntersecting)
          .sort((a, b) => a.boundingClientRect.top - b.boundingClientRect.top)[0]
        if (visible) setActive(visible.target.id as QuoteSectionId)
      },
      { rootMargin: '-20% 0px -70% 0px' }
    )
    sections.forEach(({ id }) => {
      const element = document.getElementById(id)
      if (element) observer.observe(element)
    })
    return () => observer.disconnect()
  }, [sections])

  return active
}

export const QuoteSectionNav = ({
  issues,
  sections,
}: {
  issues: SectionIssue[]
  sections: readonly QuoteSectionLink[]
}) => {
  const active = useActiveSection(sections)

  return (
    <nav aria-label="Quote sections" className="flex flex-col gap-1">
      {sections.map(section => {
        const count = issues.filter(i => i.section === section.id).length
        return (
          <a
            key={section.id}
            href={`#${section.id}`}
            onClick={e => {
              e.preventDefault()
              scrollToSection(section.id)
            }}
            aria-current={active === section.id ? 'location' : undefined}
            className={cn(
              'flex items-center gap-2.5 rounded-md border px-2.5 py-2 text-sm transition-colors',
              active === section.id
                ? 'border-border bg-card font-medium text-foreground'
                : 'border-transparent text-muted-foreground hover:bg-muted hover:text-foreground'
            )}
          >
            {count > 0 ? (
              <span
                className="flex h-5 min-w-5 items-center justify-center rounded-full bg-warning/15 px-1 text-xs font-medium text-warning"
                aria-label={`${count} to fix`}
              >
                {count}
              </span>
            ) : (
              <span
                className="flex h-5 w-5 items-center justify-center rounded-full bg-success/15 text-success"
                aria-label="Complete"
              >
                <Check className="h-3 w-3" />
              </span>
            )}
            {section.label}
          </a>
        )
      })}
    </nav>
  )
}

/** What blocks saving, each linking to its section. */
export const BeforePublishingChecklist = ({ issues }: { issues: SectionIssue[] }) => (
  <div className="flex flex-col gap-2 rounded-lg border border-border bg-card p-3">
    <span className="text-xs font-semibold">Before publishing</span>
    {issues.length === 0 ? (
      <span className="flex items-center gap-1.5 text-xs text-success">
        <Check className="h-3.5 w-3.5" />
        Ready to save
      </span>
    ) : (
      <ul className="flex flex-col gap-1.5">
        {issues.map(issue => (
          <li key={`${issue.section}-${issue.message}`}>
            <button
              type="button"
              onClick={() => scrollToSection(issue.section)}
              className="flex w-full items-start gap-1.5 text-left text-xs text-muted-foreground hover:text-foreground"
            >
              <CircleAlert className="mt-0.5 h-3.5 w-3.5 shrink-0 text-warning" aria-hidden />
              {issue.message}
            </button>
          </li>
        ))}
      </ul>
    )}
  </div>
)

/** The section links and what blocks saving: the form's own rules plus the editor's (pricing). */
export const QuoteEditorOutline = ({
  editorIssues,
  template = false,
}: {
  editorIssues: SectionIssue[]
  /** A template: no deal fields, nothing to publish. */
  template?: boolean
}) => {
  const { control } = useFormContext<QuoteFormValues>()
  const values = useWatch({ control }) as Partial<QuoteFormValues>
  const issues = [...formIssues(values, template ? DEAL_FIELDS : []), ...editorIssues]

  return (
    <div className="flex flex-col gap-4">
      <QuoteSectionNav issues={issues} sections={template ? TEMPLATE_SECTIONS : QUOTE_SECTIONS} />
      {!template && <BeforePublishingChecklist issues={issues} />}
    </div>
  )
}
