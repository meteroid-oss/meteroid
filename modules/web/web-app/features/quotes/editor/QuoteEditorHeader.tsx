import { Button } from '@md/ui'
import { ChevronRight, Eye, Save } from 'lucide-react'
import { Fragment } from 'react'
import { Link } from 'react-router'

import { QuoteStatusBadge } from '@/features/quotes/QuoteStatusBadge'
import { AutosaveIndicator } from '@/features/quotes/editor/AutosaveBanners'

import type { AutosaveStatus } from '@/features/quotes/editor/useQuoteAutosave'
import type { QuoteStatus } from '@/rpc/api/quotes/v1/models_pb'
import type { ReactNode } from 'react'

export interface EditorCrumb {
  label: string
  to?: string
}

interface QuoteEditorHeaderProps {
  /** The breadcrumb; its last step is the current page. */
  crumbs: EditorCrumb[]
  /** A heading text, or the field that edits the name. */
  title: ReactNode
  subtitle?: string
  /** Under the title, e.g. a template's description and validity. */
  details?: ReactNode
  status?: QuoteStatus
  autosaveStatus?: AutosaveStatus
  templates?: ReactNode
  onPreview: () => void
  previewDisabled: boolean
  onSave: () => void
  saveDisabled: boolean
  saveLabel: string
}

export const QuoteEditorHeader = ({
  crumbs,
  title,
  subtitle,
  details,
  status,
  autosaveStatus,
  templates,
  onPreview,
  previewDisabled,
  onSave,
  saveDisabled,
  saveLabel,
}: QuoteEditorHeaderProps) => (
  <header className="sticky top-0 z-20 -mx-6 flex flex-wrap items-center gap-3 border-b border-border bg-background px-6 py-3">
    <div className="flex min-w-0 flex-[1_1_320px] flex-col gap-0.5">
      <nav
        aria-label="Breadcrumb"
        className="flex items-center gap-1 text-xs text-muted-foreground"
      >
        {crumbs.map((crumb, index) => (
          <Fragment key={index}>
            {index > 0 && <ChevronRight className="h-3 w-3 shrink-0" aria-hidden />}
            {crumb.to ? (
              <Link to={crumb.to} className="truncate hover:text-foreground hover:underline">
                {crumb.label}
              </Link>
            ) : (
              <span className="truncate" aria-current="page">
                {crumb.label}
              </span>
            )}
          </Fragment>
        ))}
      </nav>
      <div className="flex min-w-0 flex-wrap items-center gap-x-3 gap-y-1">
        {typeof title === 'string' ? (
          <h1 className="truncate text-2xl font-semibold">{title}</h1>
        ) : (
          title
        )}
        {status !== undefined && <QuoteStatusBadge status={status} />}
        {autosaveStatus && <AutosaveIndicator status={autosaveStatus} />}
      </div>
      {subtitle && <p className="text-xs text-muted-foreground">{subtitle}</p>}
      {details}
    </div>
    <div className="flex flex-wrap items-center gap-2">
      {templates}
      <Button
        type="button"
        variant="outline"
        hasIcon
        onClick={onPreview}
        disabled={previewDisabled}
      >
        <Eye className="h-4 w-4" />
        Preview as customer
      </Button>
      <Button type="button" hasIcon onClick={onSave} disabled={saveDisabled}>
        <Save className="h-4 w-4" />
        {saveLabel}
      </Button>
    </div>
  </header>
)
