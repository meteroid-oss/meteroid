import { syntaxTree } from '@codemirror/language'
import { type Diagnostic } from '@codemirror/lint'
import { EditorState } from '@codemirror/state'
import { EditorView } from '@codemirror/view'
import Ajv, { Schema, ValidateFunction } from 'ajv'

import type { SyntaxNode } from '@lezer/common'

const getErrorPosition = (error: SyntaxError, state: EditorState) => {
  const m1 = error.message.match(/at position (\d+)/)
  if (m1) {
    return Math.min(parseInt(m1[1]!), state.doc.length)
  }
  const m2 = error.message.match(/at line (\d+) column (\d+)/)
  if (m2) {
    return Math.min(state.doc.line(parseInt(m2[1]!)).from + parseInt(m2[2]!) - 1, state.doc.length)
  }

  return 0
}

const unquote = (s: string) => {
  try {
    return JSON.parse(s)
  } catch (_) {
    return s
  }
}

const walkNode = (
  editorState: EditorState,
  node: SyntaxNode | null,
  each: (path: string, node: SyntaxNode) => void,
  path = '/'
) => {
  if (!node) {
    return
  }

  switch (node.name) {
    case 'JsonText':
      walkNode(editorState, node.firstChild, each, path)
      break
    case 'Object':
      each(path, node)

      for (let n = node.firstChild; n != null; n = n.nextSibling) {
        if (n.name == 'Property') {
          const propNameNode = n.firstChild
          const propValueNode = n.lastChild
          if (propNameNode && propValueNode) {
            const propName = unquote(editorState.sliceDoc(propNameNode.from, propNameNode.to))
            each(`${path}${propName}`, propNameNode)
            walkNode(editorState, propValueNode, each, `${path}${propName}/`)
          }
        }
      }
      break
    case 'Array':
      each(path, node)
      // eslint-disable-next-line no-case-declarations
      let idx = 0
      for (let n = node.firstChild; n != null; n = n.nextSibling) {
        if (n.name != '[' && n.name != ']') {
          each(`${path}${idx}`, node)
          walkNode(editorState, n, each, `${path}${idx}/`)
          idx++
        }
      }
      break
    default:
  }
}

const validateErrorsToDiagnostics = (
  errorSet: { [K: string]: string },
  editorState: EditorState
): Diagnostic[] => {
  if (Object.keys(errorSet).length == 0) {
    return []
  }

  const diagnostics: Diagnostic[] = []
  walkNode(editorState, syntaxTree(editorState).topNode, (path, node) => {
    if (path === '/') {
      path = ''
    }

    const msg = errorSet[path]
    if (msg) {
      diagnostics.push({
        from: node.from,
        to: node.to,
        severity: 'error',
        message: errorSet[path]!,
      })
    }
  })
  return diagnostics
}

const validate = (data: unknown, validateFn: ValidateFunction) => {
  if (validateFn(data) || !validateFn.errors) {
    return {}
  }

  const errors = validateFn.errors

  const parentInstancePath = (instancePath: string) => {
    const parts = instancePath.split('/')
    if (parts.length == 1) {
      return ''
    }
    return parts.slice(0, parts.length - 1).join('/')
  }

  const errMaps: { [k: string]: string } = {}

  for (const err of errors) {
    if (err.keyword === 'additionalProperties') {
      if (err.params.additionalProperty) {
        err.instancePath += `/${err.params.additionalProperty}`
      }
    }

    if (err.keyword === 'discriminator') {
      err.instancePath += `/${err.params.tag}`
    }

    switch (err.keyword) {
      case 'enum':
        errMaps[err.instancePath] = `${err.message}: ${err.params.allowedValues.join(', ')}`
        break
      default:
        errMaps[err.instancePath] = err.message!
    }
  }

  for (const instancePath in errMaps) {
    const pInstancePath = parentInstancePath(instancePath)
    if (pInstancePath != instancePath && errMaps[pInstancePath]) {
      delete errMaps[pInstancePath]
    }
  }

  return errMaps
}

export const jsonSchemaLinter = (options: { schema: Schema }) => {
  const ajv = new Ajv({
    strict: true,
  })
  const validateFn = ajv.compile(options.schema)

  return (view: EditorView): Diagnostic[] => {
    try {
      const object = JSON.parse(view.state.doc.toString())
      return validateErrorsToDiagnostics(validate(object, validateFn), view.state)
    } catch (e) {
      if (!(e instanceof SyntaxError)) {
        throw e
      }

      const pos = getErrorPosition(e, view.state)
      return [
        {
          from: pos,
          severity: 'error',
          message: e.message,
          to: pos,
        },
      ]
    }
  }
}
