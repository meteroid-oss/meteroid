import { Code } from '@connectrpc/connect'

import { useQuery } from '@/lib/connectrpc'
import { listEmailVariables } from '@/rpc/api/customeremails/v1/customeremails-CustomerEmailsService_connectquery'

interface Props {
  onInsert: (variable: string) => void
}

/** The `{{variables}}` a reminder body may use, one chip each; a click inserts it. */
export const VariableChips = ({ onInsert }: Props) => {
  const query = useQuery(
    listEmailVariables,
    {},
    {
      staleTime: Infinity,
      retry: (count, err) => err.code !== Code.Unimplemented && count < 2,
    }
  )
  const variables = query.data?.variables ?? []
  if (variables.length === 0) return null

  return (
    <div className="flex flex-wrap items-center gap-1">
      <span className="mr-1 text-[11px] text-muted-foreground">Insert</span>
      {variables.map(v => (
        <button
          key={v.name}
          type="button"
          title={v.example ? `${v.kind}: ${v.example}` : v.kind}
          onClick={() => onInsert(v.name)}
          className="rounded border border-border bg-muted/40 px-1.5 py-0.5 font-mono text-[11px] text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
        >
          {`{{${v.name}}}`}
        </button>
      ))}
    </div>
  )
}
