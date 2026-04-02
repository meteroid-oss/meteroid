import { Button, Input, Label, Popover, PopoverContent, PopoverTrigger, Select, SelectContent, SelectItem, SelectTrigger, SelectValue, cn } from '@md/ui'
import { Check, Copy, Wrench } from 'lucide-react'
import { useCallback, useState } from 'react'

const BASE62_CHARS = '0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz'
const MASK_128 = (1n << 128n) - 1n

const ID_PREFIXES = [
  'org_', 'ten_', 'cus_', 'sub_', 'inv_', 'plan_', 'plv_', 'prd_',
  'price_', 'bm_', 'cou_', 'add_', 'pf_', 'evt_', 'pay_', 'ive_',
  'crn_', 'pm_', 'ctn_', 'ctr_', 'doc_', 'che_', 'pri_', 'sev_',
] as const

function base62Decode(s: string): bigint {
  let result = 0n
  for (const ch of s) {
    const idx = BASE62_CHARS.indexOf(ch)
    if (idx === -1) throw new Error(`Invalid base62 character: ${ch}`)
    result = result * 62n + BigInt(idx)
  }
  return result
}

function base62Encode(n: bigint): string {
  if (n === 0n) return '0'
  let result = ''
  let val = n
  while (val > 0n) {
    result = BASE62_CHARS[Number(val % 62n)] + result
    val = val / 62n
  }
  return result
}

function rotateRight128(val: bigint, bits: number): bigint {
  const v = val & MASK_128
  return ((v >> BigInt(bits)) | (v << BigInt(128 - bits))) & MASK_128
}

function rotateLeft128(val: bigint, bits: number): bigint {
  const v = val & MASK_128
  return ((v << BigInt(bits)) | (v >> BigInt(128 - bits))) & MASK_128
}

function bigintToUuid(n: bigint): string {
  const hex = n.toString(16).padStart(32, '0')
  return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20, 32)}`
}

function uuidToBigint(uuid: string): bigint {
  return BigInt('0x' + uuid.replace(/-/g, ''))
}

const UUID_RE = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i

function detectInputType(input: string): 'uuid' | 'prefixed' | 'unknown' {
  if (UUID_RE.test(input)) return 'uuid'
  if (input.includes('_') && input.length > 4) return 'prefixed'
  return 'unknown'
}

function prefixedToUuid(prefixedId: string): string {
  const base62Part = prefixedId.slice(prefixedId.lastIndexOf('_') + 1)
  const decoded = base62Decode(base62Part)
  const uuid = rotateRight128(decoded, 67)
  return bigintToUuid(uuid)
}

function uuidToPrefixed(uuid: string, prefix: string): string {
  const n = uuidToBigint(uuid)
  const rotated = rotateLeft128(n, 67)
  return prefix + base62Encode(rotated)
}

function CopyButton({ text }: { text: string }) {
  const [copied, setCopied] = useState(false)

  const copy = useCallback(() => {
    navigator.clipboard.writeText(text)
    setCopied(true)
    setTimeout(() => setCopied(false), 1500)
  }, [text])

  return (
    <button
      onClick={copy}
      className="text-muted-foreground hover:text-foreground transition-colors shrink-0"
    >
      {copied ? <Check className="h-3.5 w-3.5 text-green-500" /> : <Copy className="h-3.5 w-3.5" />}
    </button>
  )
}

function IdConverter() {
  const [input, setInput] = useState('')
  const [prefix, setPrefix] = useState('org_')

  const inputType = input.trim() ? detectInputType(input.trim()) : null

  let result: string | null = null
  let error: string | null = null

  if (input.trim() && inputType) {
    try {
      if (inputType === 'uuid') {
        result = uuidToPrefixed(input.trim(), prefix)
      } else if (inputType === 'prefixed') {
        result = prefixedToUuid(input.trim())
      }
    } catch {
      error = 'Invalid ID format'
    }
  }

  return (
    <div className="space-y-3">
      <div className="space-y-1.5">
        <Label className="text-xs text-muted-foreground">Input ID</Label>
        <Input
          placeholder="UUID or prefixed ID..."
          value={input}
          onChange={e => setInput(e.target.value)}
          className="h-8 text-xs font-mono"
        />
      </div>

      {inputType === 'uuid' && (
        <div className="space-y-1.5">
          <Label className="text-xs text-muted-foreground">Prefix</Label>
          <Select value={prefix} onValueChange={setPrefix}>
            <SelectTrigger className="h-8 text-xs font-mono">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {ID_PREFIXES.map(p => (
                <SelectItem key={p} value={p} className="text-xs font-mono">
                  {p}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
      )}

      {error && <p className="text-xs text-destructive">{error}</p>}

      {result && (
        <div className="space-y-1.5">
          <Label className="text-xs text-muted-foreground">
            {inputType === 'uuid' ? 'Prefixed ID' : 'UUID'}
          </Label>
          <div className="flex items-center gap-2 rounded-md border border-border bg-muted px-2.5 py-1.5">
            <span className="text-xs font-mono break-all flex-1 select-all">{result}</span>
            <CopyButton text={result} />
          </div>
        </div>
      )}
    </div>
  )
}

export const AdminToolbox = () => {
  const [open, setOpen] = useState(false)

  return (
    <div className="fixed bottom-4 right-4 z-50">
      <Popover open={open} onOpenChange={setOpen}>
        <PopoverTrigger asChild>
          <Button
            variant="outline"
            size="icon"
            className={cn(
              'h-10 w-10 rounded-full shadow-lg border-border bg-background',
              open && 'bg-accent'
            )}
          >
            <Wrench className="h-4 w-4" />
          </Button>
        </PopoverTrigger>
        <PopoverContent side="top" align="end" className="w-80 p-3">
          <div className="space-y-2">
            <h4 className="text-sm font-medium">Admin Tools</h4>
            <IdConverter />
          </div>
        </PopoverContent>
      </Popover>
    </div>
  )
}
