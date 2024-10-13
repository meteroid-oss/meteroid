module.exports = {
  root: true,
  // load the config from `eslint-config-meteroid`
  extends: ['meteroid'],
  rules: {
    'import/no-cycle': 'error',
    '@typescript-eslint/no-unused-vars': [
      'error',
      {
        argsIgnorePattern: '^_',
        varsIgnorePattern: '^_',
      },
    ],
  },
}
