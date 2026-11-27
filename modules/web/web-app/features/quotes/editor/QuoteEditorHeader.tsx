import {
  Button,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
  Tooltip,
  TooltipContent,
  TooltipTrigger,
  cn,
} from '@md/ui'
import { ChevronDown, X } from 'lucide-react'
import { Link } from 'react-router'

import { QuoteStatusBadge } from '@/features/quotes/QuoteStatusBadge'
import { AutosaveIndicator } from '@/features/quotes/editor/AutosaveBanners'
import { editorSections, useEditorIssues } from '@/features/quotes/editor/QuoteSectionNav'
import { QUOTE_STEPS, stepOfSection } from '@/features/quotes/editor/editorSections'

import type {
  QuoteSectionId,
  QuoteStep,
  SectionIssue,
} from '@/features/quotes/editor/editorSections'
import type { AutosaveStatus } from '@/features/quotes/editor/useQuoteAutosave'
import type { QuoteStatus } from '@/rpc/api/quotes/v1/models_pb'
import type { ReactNode } from 'react'

export interface EditorCrumb {
  label: string
  to?: string
}

interface QuoteEditorHeaderProps {
  /** Where the editor sits; its last step names it, the one before is where ✕ returns to. */
  crumbs: EditorCrumb[]
  /** Beside the name: the field that edits a template's name. A quote shows none; its customer
   *  and plan are in the form. */
  title?: ReactNode
  status?: QuoteStatus
  autosaveStatus?: AutosaveStatus
  templates?: ReactNode
  /** Whether the side preview is shown, and its toggle. */
  previewOpen: boolean
  onTogglePreview: () => void
  /** The whole document, as the customer sees it. */
  onPreview: () => void
  previewDisabled: boolean
  /** Editing a template: no deal fields, so other sections and rules. */
  template?: boolean
  /** What the editor itself blocks saving on (pricing), besides the form's own rules. */
  editorIssues: SectionIssue[]
  /** The step shown, and moving between steps; any step can be opened at any time. */
  step: QuoteStep
  onStepChange: (step: QuoteStep) => void
  /** Until the customer is chosen there is only the first step. */
  stepsLocked?: boolean
  /** Opens a section: its step, then scrolled to it. */
  onGoToSection: (section: QuoteSectionId) => void
}

/** The editor's top bar: close and name on the left, the steps in the middle, preview and
 *  actions on the right. Saving is the last step's own action, under the form. */
export const QuoteEditorHeader = ({
  crumbs,
  title,
  status,
  autosaveStatus,
  templates,
  previewOpen,
  onTogglePreview,
  onPreview,
  previewDisabled,
  template = false,
  editorIssues,
  step,
  onStepChange,
  stepsLocked = false,
  onGoToSection,
}: QuoteEditorHeaderProps) => {
  const current = crumbs[crumbs.length - 1]
  const back = crumbs[crumbs.length - 2]?.to
  const sections = editorSections(template)
  const issues = useEditorIssues(editorIssues, template)

  return (
    // Three columns, the outer two equal, so the steps sit in the middle of the bar.
    <header className="sticky top-0 z-10 grid h-12 shrink-0 grid-cols-[minmax(0,1fr)_auto_minmax(0,1fr)] items-center gap-3 border-b border-border bg-background px-3">
      <div className="flex min-w-0 items-center gap-3">
        {back && (
          <>
            <Tooltip>
              <TooltipTrigger asChild>
                <Link
                  to={back}
                  aria-label="Close"
                  className="flex size-7 shrink-0 items-center justify-center rounded-[4px] text-muted-foreground transition-colors hover:bg-accent hover:text-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                >
                  <X className="h-4 w-4" />
                </Link>
              </TooltipTrigger>
              <TooltipContent side="bottom">Close</TooltipContent>
            </Tooltip>
            <span aria-hidden className="h-4 w-px shrink-0 bg-border" />
          </>
        )}

        <div className="flex min-w-0 flex-1 items-center gap-2">
          <h1 className="shrink-0 text-[13px] font-medium text-foreground">{current?.label}</h1>
          {status !== undefined && <QuoteStatusBadge status={status} />}
          {title && <div className="min-w-0">{title}</div>}
          {autosaveStatus && (
            <span className="shrink-0">
              <AutosaveIndicator status={autosaveStatus} />
            </span>
          )}
        </div>
      </div>

      <nav aria-label="Steps" className="flex items-center gap-1">
        {QUOTE_STEPS.map(({ id, label }) => {
          const count = issues.filter(issue => stepOfSection(issue.section) === id).length
          const locked = stepsLocked && id !== 1
          return (
            <button
              key={id}
              type="button"
              onClick={() => onStepChange(id)}
              disabled={locked}
              aria-current={step === id ? 'step' : undefined}
              className={cn(
                'flex h-7 items-center gap-2 rounded-[5px] px-2.5 text-[13px] font-medium transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-40',
                step === id
                  ? 'bg-foreground/[0.08] text-foreground'
                  : 'text-muted-foreground hover:bg-foreground/[0.05] hover:text-foreground'
              )}
            >
              <span
                className={cn(
                  'flex size-4 items-center justify-center rounded-full text-[10px] tabular-nums',
                  step === id ? 'bg-foreground text-background' : 'bg-foreground/[0.1]'
                )}
              >
                {id}
              </span>
              <span className="hidden xl:inline">{label}</span>
              {count > 0 && !locked && (
                <span
                  className="text-[11px] tabular-nums text-warning"
                  aria-label={`${count} to fix`}
                >
                  {count}
                </span>
              )}
            </button>
          )
        })}
      </nav>

      <div className="flex min-w-0 items-center justify-end gap-2">
        {templates}
        <Button type="button" variant="secondary" size="sm" onClick={onTogglePreview}>
          {previewOpen ? 'Hide preview' : 'Preview'}
        </Button>
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button type="button" variant="secondary" size="sm" className="gap-1.5 pr-2">
              Actions
              <ChevronDown className="h-3.5 w-3.5 text-muted-foreground" aria-hidden />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end" className="w-52">
            <DropdownMenuItem disabled={previewDisabled} onSelect={onPreview}>
              Preview as customer
            </DropdownMenuItem>
            <DropdownMenuSeparator />
            <DropdownMenuLabel>Jump to</DropdownMenuLabel>
            {sections.map(section => (
              <DropdownMenuItem
                key={section.id}
                disabled={stepsLocked && section.step !== 1}
                onSelect={() => onGoToSection(section.id)}
              >
                <span className="w-4 text-xs tabular-nums text-muted-foreground">
                  {section.step}
                </span>
                {section.label}
              </DropdownMenuItem>
            ))}
          </DropdownMenuContent>
        </DropdownMenu>
      </div>
    </header>
  )
}
