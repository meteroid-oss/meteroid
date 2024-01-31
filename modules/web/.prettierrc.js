/**
 * {@type require('prettier').Config}
 */
module.exports = {
  $schema: 'http://json.schemastore.org/prettierrc',

  printWidth: 100,
  semi: false,
  singleQuote: true,
  proseWrap: 'preserve',
  useTabs: false,
  tabWidth: 2,
  arrowParens: 'avoid',
  bracketSpacing: true,
  cursorOffset: -1,
  insertPragma: false,
  requirePragma: false,
  bracketSameLine: false,
  quoteProps: 'as-needed',
  trailingComma: 'es5',

  // trailingComma: 'none',
  // TODO
  // importOrder: [
  // 	// external packages
  // 	'^([A-Za-z]|@[^s/])',
  // 	// meteroid packages
  // 	'^@md/(interface|client|ui)(/.*)?$',
  // 	// this package
  // 	'^~/',
  // 	// relative
  // 	'^\\.'
  // ],
  // importOrderSortSpecifiers: true,
  // importOrderParserPlugins: ['importAssertions', 'typescript', 'jsx'],
  // pluginSearchDirs: false,
  // plugins: ['@trivago/prettier-plugin-sort-imports', 'prettier-plugin-tailwindcss'],
  // tailwindConfig: './packages/ui/tailwind.config.js'
}
