import {
  Badge,
  Button,
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
  Input,
  Textarea,
} from '@md/ui'
import { ArrowDown, ArrowUp, ChevronRight, Lock, Plus, Trash2 } from 'lucide-react'

import { EditorSection } from '@/features/quotes/editor/EditorSection'
import {
  canMoveSection,
  isBeforePricing,
  moveSection,
  readingOrder,
} from '@/features/quotes/editor/documentBlocks'
import { QuoteSectionPlacement } from '@/rpc/api/quotes/v1/models_pb'

import type { QuoteV2State, SectionState } from '@/features/quotes/editor/quoteV2'
import type { Dispatch, ReactNode, SetStateAction } from 'react'

const summary = (text: string | undefined) => {
  const trimmed = text?.trim() ?? ''
  if (!trimmed) return 'Empty'
  return trimmed.length > 80 ? `${trimmed.slice(0, 80)}…` : trimmed
}

interface BlockProps {
  title: ReactNode
  kind?: string
  preview: string
  defaultOpen: boolean
  controls?: ReactNode
  children: ReactNode
}

const Block = ({ title, kind, preview, defaultOpen, controls, children }: BlockProps) => (
  <Collapsible defaultOpen={defaultOpen} className="rounded-lg border border-border">
    <div className="flex items-center gap-2 p-2 pl-3">
      <CollapsibleTrigger className="group flex min-w-0 flex-1 items-center gap-2 text-left">
        <ChevronRight className="h-4 w-4 shrink-0 text-muted-foreground transition-transform group-data-[state=open]:rotate-90" />
        <span className="truncate text-sm font-semibold">{title}</span>
        {kind && (
          <Badge variant="outline" size="sm" className="font-normal text-muted-foreground">
            {kind}
          </Badge>
        )}
        <span className="truncate text-xs text-muted-foreground group-data-[state=open]:hidden">
          {preview}
        </span>
      </CollapsibleTrigger>
      {controls}
    </div>
    <CollapsibleContent className="flex flex-col gap-3 border-t border-border p-3">
      {children}
    </CollapsibleContent>
  </Collapsible>
)

interface DocumentSectionProps {
  sections: SectionState[]
  setV2: Dispatch<SetStateAction<QuoteV2State>>
}

export const DocumentSection = ({ sections, setV2 }: DocumentSectionProps) => {
  const ordered = readingOrder(sections)
  const beforeCount = ordered.filter(isBeforePricing).length

  const setSections = (update: (list: SectionState[]) => SectionState[]) =>
    setV2(prev => ({ ...prev, sections: update(prev.sections) }))

  // Edits address the reading order; it's also the order the sections are saved in.
  const updateAt = (index: number, patch: Partial<SectionState>) =>
    setSections(list => readingOrder(list).map((s, i) => (i === index ? { ...s, ...patch } : s)))

  const sectionBlock = (section: SectionState, index: number) => (
    <Block
      key={index}
      title={section.title.trim() || 'Untitled section'}
      kind="Markdown"
      preview={summary(section.body)}
      defaultOpen={!section.body.trim()}
      controls={
        <div className="flex shrink-0 items-center">
          <Button
            type="button"
            variant="ghost"
            size="icon"
            className="h-7 w-7"
            disabled={!canMoveSection(sections, index, -1)}
            onClick={() => setSections(list => moveSection(list, index, -1))}
            aria-label={`Move ${section.title || 'section'} up`}
          >
            <ArrowUp className="h-3.5 w-3.5" />
          </Button>
          <Button
            type="button"
            variant="ghost"
            size="icon"
            className="h-7 w-7"
            disabled={!canMoveSection(sections, index, 1)}
            onClick={() => setSections(list => moveSection(list, index, 1))}
            aria-label={`Move ${section.title || 'section'} down`}
          >
            <ArrowDown className="h-3.5 w-3.5" />
          </Button>
          <Button
            type="button"
            variant="ghost"
            size="icon"
            className="h-7 w-7"
            onClick={() => setSections(list => readingOrder(list).filter((_, i) => i !== index))}
            aria-label={`Remove ${section.title || 'section'}`}
          >
            <Trash2 className="h-3.5 w-3.5" />
          </Button>
        </div>
      }
    >
      <Input
        value={section.title}
        onChange={e => updateAt(index, { title: e.target.value })}
        placeholder="Title"
        aria-label="Section title"
      />
      <Textarea
        value={section.body}
        onChange={e => updateAt(index, { body: e.target.value })}
        placeholder="Markdown"
        aria-label="Section content"
        rows={5}
      />
    </Block>
  )

  return (
    <EditorSection id="quote-document" title="Document">
      <div className="flex flex-col gap-2">
        {ordered.slice(0, beforeCount).map((section, i) => sectionBlock(section, i))}

        <div className="flex items-center gap-2 rounded-lg border border-dashed border-border bg-muted/40 p-3">
          <Lock className="h-4 w-4 text-muted-foreground" aria-hidden />
          <span className="text-sm font-semibold">Pricing</span>
        </div>

        {ordered.slice(beforeCount).map((section, i) => sectionBlock(section, beforeCount + i))}

        <Button
          type="button"
          variant="ghost"
          size="sm"
          hasIcon
          className="self-start"
          onClick={() =>
            setSections(list => [
              ...readingOrder(list),
              { title: '', body: '', placement: QuoteSectionPlacement.AFTER_PRICING },
            ])
          }
        >
          <Plus className="h-4 w-4" />
          Add section
        </Button>
      </div>
    </EditorSection>
  )
}
