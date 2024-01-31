import { json } from '@codemirror/lang-json'
import { linter } from '@codemirror/lint'
import { Extension } from '@codemirror/state'
import { githubDark, githubLight } from '@uiw/codemirror-theme-github'
import CodeMirror, { ReactCodeMirrorRef } from '@uiw/react-codemirror'
import { Schema } from 'ajv'
import { forwardRef, useMemo } from 'react'

import { jsonSchemaLinter } from '@/utils/editor/jsonSchemaLinter'
import { useTheme } from 'providers/ThemeProvider'

interface JsonEditorProps {
  onChange: (value: string) => void
  value?: string
  placeholder?: string
  onBlur?: () => void
  schema?: Schema
}

export const JsonEditor = forwardRef<ReactCodeMirrorRef, JsonEditorProps>(
  ({ onChange, value, placeholder, onBlur, schema }: JsonEditorProps, ref) => {
    const isDarkMode = useTheme().isDarkMode
    const extensions = useMemo(() => {
      const extensions: Extension[] = [json()]
      if (schema) {
        extensions.push(linter(jsonSchemaLinter({ schema })))
      }
      return extensions
    }, [schema])

    return (
      <CodeMirror
        extensions={extensions}
        theme={isDarkMode ? githubDark : githubLight}
        onChange={onChange}
        value={value}
        placeholder={placeholder}
        onBlur={onBlur}
        ref={ref}
      />
    )
  }
)
