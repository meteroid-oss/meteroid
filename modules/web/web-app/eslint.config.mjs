import meteroid from 'eslint-config-meteroid'

export default [
  ...meteroid,
  {
    rules: {
      'import/no-cycle': 'error',
      'import/no-restricted-paths': [
        'error',
        {
          basePath: import.meta.dirname,
          zones: [
            {
              target: '.',
              from: '../../../ee',
              message: 'Core never imports EE code: an edition module contributes it (see edition/types.ts).',
            },
          ],
        },
      ],
      '@typescript-eslint/no-unused-vars': [
        'error',
        {
          argsIgnorePattern: '^_',
          varsIgnorePattern: '^_',
          caughtErrors: 'none',
        },
      ],
      'no-restricted-syntax': [
        'error',
        {
          selector:
            ":matches(CallExpression[callee.name=/^use(Layout)?Effect$/], CallExpression[callee.property.name=/^use(Layout)?Effect$/]) > ArrowFunctionExpression[body.type!='BlockStatement']",
          message:
            'Give the effect a block body: whatever an expression body returns, React calls as its cleanup.',
        },
      ],
    },
  },
]
