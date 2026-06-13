import meteroid from 'eslint-config-meteroid'

export default [
  ...meteroid,
  {
    rules: {
      'import/no-cycle': 'error',
      '@typescript-eslint/no-unused-vars': [
        'error',
        {
          argsIgnorePattern: '^_',
          varsIgnorePattern: '^_',
          caughtErrors: 'none',
        },
      ],
    },
  },
]
