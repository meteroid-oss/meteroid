import { Fragment, ReactNode } from 'react'

import { QuoteSection, QuoteSectionPlacement } from '@/rpc/api/quotes/v1/models_pb'

const INLINE =
  /(\*\*[^*]+\*\*|__[^_]+__|\*[^*\s][^*]*\*|_[^_\s][^_]*_|`[^`]+`|\[[^\]]+\]\([^)\s]+\))/g

const safeHref = (href: string) => (/^(https?:|mailto:)/i.test(href) ? href : undefined)

const renderInline = (text: string): ReactNode[] =>
  text.split(INLINE).map((part, i) => {
    if (!part) return null
    if (/^(\*\*|__).+(\*\*|__)$/.test(part)) return <strong key={i}>{part.slice(2, -2)}</strong>
    if (/^`.+`$/.test(part))
      return (
        <code key={i} className="rounded bg-muted px-1 text-[0.9em]">
          {part.slice(1, -1)}
        </code>
      )
    const link = part.match(/^\[([^\]]+)\]\(([^)\s]+)\)$/)
    if (link) {
      const href = safeHref(link[2])
      return href ? (
        <a key={i} href={href} target="_blank" rel="noopener noreferrer" className="underline">
          {link[1]}
        </a>
      ) : (
        <Fragment key={i}>{link[1]}</Fragment>
      )
    }
    if (/^([*_]).+\1$/.test(part)) return <em key={i}>{part.slice(1, -1)}</em>
    return <Fragment key={i}>{part}</Fragment>
  })

type Block =
  | { kind: 'heading'; level: number; text: string }
  | { kind: 'ul' | 'ol'; items: string[] }
  | { kind: 'p'; lines: string[] }

const parseBlocks = (source: string): Block[] => {
  const blocks: Block[] = []
  for (const raw of source.replace(/\r\n/g, '\n').split('\n')) {
    const line = raw.trimEnd()
    const last = blocks[blocks.length - 1]
    const heading = line.match(/^(#{1,6})\s+(.*)$/)
    const bullet = line.match(/^\s*[-*+]\s+(.*)$/)
    const numbered = line.match(/^\s*\d+[.)]\s+(.*)$/)
    if (!line.trim()) {
      blocks.push({ kind: 'p', lines: [] })
    } else if (heading) {
      blocks.push({ kind: 'heading', level: heading[1].length, text: heading[2] })
    } else if (bullet || numbered) {
      const kind = bullet ? 'ul' : 'ol'
      const item = (bullet ?? numbered)![1]
      if (last?.kind === kind) last.items.push(item)
      else blocks.push({ kind, items: [item] })
    } else if (last?.kind === 'p') {
      last.lines.push(line)
    } else {
      blocks.push({ kind: 'p', lines: [line] })
    }
  }
  return blocks.filter(b => b.kind !== 'p' || b.lines.length > 0)
}

export const QuoteMarkdown = ({ source, className }: { source: string; className?: string }) => (
  <div className={className ?? 'space-y-2 text-sm text-muted-foreground'}>
    {parseBlocks(source).map((block, i) => {
      switch (block.kind) {
        case 'heading':
          return (
            <p
              key={i}
              className={
                block.level <= 2 ? 'font-semibold text-foreground' : 'font-medium text-foreground'
              }
            >
              {renderInline(block.text)}
            </p>
          )
        case 'ul':
          return (
            <ul key={i} className="list-disc pl-5 space-y-1">
              {block.items.map((item, j) => (
                <li key={j}>{renderInline(item)}</li>
              ))}
            </ul>
          )
        case 'ol':
          return (
            <ol key={i} className="list-decimal pl-5 space-y-1">
              {block.items.map((item, j) => (
                <li key={j}>{renderInline(item)}</li>
              ))}
            </ol>
          )
        case 'p':
          return (
            <p key={i}>
              {block.lines.map((l, j) => (
                <Fragment key={j}>
                  {j > 0 && <br />}
                  {renderInline(l)}
                </Fragment>
              ))}
            </p>
          )
      }
    })}
  </div>
)

export const QuoteSections = ({
  sections,
  placement,
}: {
  sections: QuoteSection[] | undefined
  placement: QuoteSectionPlacement
}) => {
  const matching = (sections ?? []).filter(s => s.placement === placement)
  if (matching.length === 0) return null
  return (
    <>
      {matching.map((section, i) => (
        <div key={i} className="mb-8">
          {section.title && <h3 className="font-semibold text-foreground mb-2">{section.title}</h3>}
          <QuoteMarkdown source={section.body} />
        </div>
      ))}
    </>
  )
}
