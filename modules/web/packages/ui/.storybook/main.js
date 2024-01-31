const { mergeConfig } = require('vite')
const { nodeResolve } = require('@rollup/plugin-node-resolve')
const path = require('path')

module.exports = {
  stories: ['../src/**/*.stories.@(ts|tsx|mdx)'],
  addons: [
    '@storybook/addon-links',
    '@storybook/addon-essentials',
    '@storybook/addon-interactions',
    {
      name: '@storybook/addon-postcss',
      options: {
        postcssLoaderOptions: {
          implementation: require('postcss'),
        },
      },
    },
  ],
  framework: {
    name: '@storybook/react-vite',
    options: {
      builder: {
        viteConfigPath: 'vite.config.ts',
      },
    },
  },
  features: {
    storyStoreV7: true,
  },
  core: {
    disableTelemetry: true,
  },
  async viteFinal(config) {
    return mergeConfig(config, {
      define: { 'process.env': {} },
      plugins: [
        nodeResolve({
          extensions: ['.tsx', '.ts'],
        }),
      ],
      resolve: {
        alias: {
          '@ui': path.resolve(__dirname, '../src'),
        },
      },
    })
  },
  docs: {
    autodocs: true,
  },
}
