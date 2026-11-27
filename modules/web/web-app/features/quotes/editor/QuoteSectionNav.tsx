import { CircleAlert } from 'lucide-react'
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

/** The form's sections, as the editor shows them for a quote or a template. */
export const editorSections = (template: boolean): readonly QuoteSectionLink[] =>
  template ? TEMPLATE_SECTIONS : QUOTE_SECTIONS

/** What blocks saving: the form's own rules plus the editor's (pricing), by section. */
export const useEditorIssues = (editorIssues: SectionIssue[], template: boolean) => {
  const { control } = useFormContext<QuoteFormValues>()
  const values = useWatch({ control }) as Partial<QuoteFormValues>
  return [...formIssues(values, template ? DEAL_FIELDS : []), ...editorIssues]
}

/** What blocks saving, each item taking the user to its section (switching steps if need be). */
export const IssueList = ({
  issues,
  onSelect,
}: {
  issues: SectionIssue[]
  onSelect: (section: QuoteSectionId) => void
}) => (
  <ul className="flex flex-col gap-1.5">
    {issues.map(issue => (
      <li key={`${issue.section}-${issue.message}`}>
        <button
          type="button"
          onClick={() => onSelect(issue.section)}
          className="flex w-full items-start gap-1.5 text-left text-[13px] text-foreground/80 hover:text-foreground"
        >
          <CircleAlert className="mt-0.5 h-3.5 w-3.5 shrink-0 text-warning" aria-hidden />
          {issue.message}
        </button>
      </li>
    ))}
  </ul>
)
