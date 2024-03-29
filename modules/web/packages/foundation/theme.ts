import { promises as fs } from 'fs'

import { darkColorsVars, lightColorsVars } from './src/tokens/colors'

const lightVars = Object.entries(lightColorsVars)
  .map(([key, value]) => `${key}: ${value};`)
  .join('\n')

const darkVars = Object.entries(darkColorsVars)
  .map(([key, value]) => `${key}: ${value};`)
  .join('\n')

const themeCss = `
/* DO NOT EDIT THIS FILE IS AUTO GENERATED */
body.light {
  ${lightVars}
}

body.dark {
  ${darkVars}
}
`

;(async () => {
  try {
    await fs.writeFile('./src/base/theme.css', themeCss)
    console.log('Colors tokens written successfully.')
  } catch (error) {
    console.error('Error writing colors tokens file:', error)
  }
})()
