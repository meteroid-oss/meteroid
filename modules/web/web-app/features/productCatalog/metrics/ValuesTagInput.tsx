import { Badge, Input } from '@md/ui'
import { XIcon } from 'lucide-react'
import { forwardRef, useRef, useState } from 'react'

interface ValuesTagInputProps {
  value: string[] | undefined
  onChange: (value: string[]) => void
  placeholder?: string
}

export const ValuesTagInput = forwardRef<HTMLDivElement, ValuesTagInputProps>(
  ({ value = [], onChange, placeholder = 'Type and press Enter' }, ref) => {
    const [inputValue, setInputValue] = useState('')
    const inputRef = useRef<HTMLInputElement>(null)

    const addValue = (val: string) => {
      const trimmed = val.trim()
      if (trimmed && !value.includes(trimmed)) {
        onChange([...value, trimmed])
      }
      setInputValue('')
    }

    const removeValue = (index: number) => {
      onChange(value.filter((_, i) => i !== index))
    }

    const handleKeyDown = (e: React.KeyboardEvent<HTMLInputElement>) => {
      if (e.key === 'Enter' || e.key === ',') {
        e.preventDefault()
        addValue(inputValue)
      } else if (e.key === 'Backspace' && inputValue === '' && value.length > 0) {
        removeValue(value.length - 1)
      }
    }

    const handleBlur = () => {
      if (inputValue.trim()) {
        addValue(inputValue)
      }
    }

    return (
      <div
        ref={ref}
        className="flex flex-wrap  cursor-text"
        onClick={() => inputRef.current?.focus()}
      >
        {value.map((val, index) => (
          <Badge key={index} variant="secondary" className="gap-1 pr-1">
            {val}
            <button
              type="button"
              onClick={e => {
                e.stopPropagation()
                removeValue(index)
              }}
              className="hover:bg-muted rounded-sm"
            >
              <XIcon className="h-3 w-3" />
            </button>
          </Badge>
        ))}
        <Input
          ref={inputRef}
          type="text"
          value={inputValue}
          onChange={e => setInputValue(e.target.value)}
          onKeyDown={handleKeyDown}
          onBlur={handleBlur}
          placeholder={value.length === 0 ? placeholder : ''}
          className="flex-1 min-w-[80px]  outline-none text-sm"
        />
      </div>
    )
  }
)
