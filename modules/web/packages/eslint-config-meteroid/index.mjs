import js from '@eslint/js'
import prettier from 'eslint-config-prettier'
import importPlugin from 'eslint-plugin-import'
import react from 'eslint-plugin-react'
import reactHooks from 'eslint-plugin-react-hooks'
import unusedImports from 'eslint-plugin-unused-imports'
import globals from 'globals'
import tseslint from 'typescript-eslint'

// Shared Meteroid ESLint flat config (ESLint v9+).
// Consumed by `import meteroid from 'eslint-config-meteroid'` in each package's
// `eslint.config.mjs`.
export default tseslint.config(
  {
    ignores: [
      '**/dist/**',
      '**/build/**',
      '**/.next/**',
      '**/public/**',
      '**/target/**',
      '**/node_modules/**',
      '**/*.lock',
      '**/*-lock.yaml',
    ],
  },
  js.configs.recommended,
  ...tseslint.configs.recommended,
  importPlugin.flatConfigs.recommended,
  importPlugin.flatConfigs.typescript,
  react.configs.flat.recommended,
  {
    languageOptions: {
      ecmaVersion: 2020,
      sourceType: 'module',
      globals: { ...globals.node, ...globals.browser },
      parserOptions: { ecmaFeatures: { jsx: true } },
    },
    plugins: {
      'react-hooks': reactHooks,
      'unused-imports': unusedImports,
    },
    settings: {
      'import/resolver': {
        typescript: { project: ['./**/tsconfig.json'] },
      },
      react: { version: 'detect' },
    },
    rules: {
      // react-hooks recommended
      'react-hooks/rules-of-hooks': 'error',
      'react-hooks/exhaustive-deps': 'warn',
      // project preferences (ported from the legacy `.eslintrc` config)
      '@typescript-eslint/no-non-null-assertion': 'off',
      '@typescript-eslint/no-empty-function': 'off',
      '@typescript-eslint/no-require-imports': 'warn',
      // not enforced under the previous (typescript-eslint v6) ruleset
      '@typescript-eslint/no-unused-expressions': 'off',
      // allow the shadcn-style empty interfaces (`interface Props extends X {}`)
      '@typescript-eslint/no-empty-object-type': 'off',
      // redundant with TypeScript and false-positives on type-only namespace
      // members under the typescript import resolver
      'import/namespace': 'off',
      // keep the v6 default of ignoring unused `catch` bindings
      '@typescript-eslint/no-unused-vars': [
        'warn',
        { argsIgnorePattern: '^_', varsIgnorePattern: '^_', caughtErrors: 'none' },
      ],
      'react/react-in-jsx-scope': 'off',
      'react/prop-types': 'off',
      'react/display-name': 'off',
      'react/jsx-curly-brace-presence': [
        'warn',
        { props: 'never', children: 'never', propElementValues: 'always' },
      ],
      'unused-imports/no-unused-imports': 'error',
      'import/no-unresolved': [2, { caseSensitive: false }],
      'import/order': [
        'error',
        {
          groups: [
            'builtin',
            'external',
            'internal',
            'parent',
            'sibling',
            'index',
            'object',
            'type',
          ],
          'newlines-between': 'always',
          alphabetize: { order: 'asc' },
        },
      ],
    },
  },
  prettier,
)
