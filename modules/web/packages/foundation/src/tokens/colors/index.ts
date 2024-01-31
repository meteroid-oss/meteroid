import { blackDark, blackLight } from './black'
import { dangerDark, dangerLight } from './danger'
import { mauveDark, mauveLight } from './mauve'
import { neutralDark, neutralLight } from './neutral'
import { primaryDark, primaryLight } from './primary'
import { purpleDark, purpleLight } from './purple'
import { secondaryDark, secondaryLight } from './secondary'
import { successDark, successLight } from './success'
import { warningDark, warningLight } from './warning'
import { whiteDark, whiteLight } from './white'

export type ColorSet = Record<string, string>

const mergeColors = (...colorSets: ColorSet[]): ColorSet => {
  return Object.assign({}, ...colorSets)
}

const lightColors: ColorSet = mergeColors(
  whiteLight,
  blackLight,
  primaryLight,
  secondaryLight,
  neutralLight,
  successLight,
  warningLight,
  dangerLight,
  mauveLight,
  purpleLight
)

const darkColors: ColorSet = mergeColors(
  whiteDark,
  blackDark,
  primaryDark,
  secondaryDark,
  neutralDark,
  successDark,
  warningDark,
  dangerDark,
  mauveDark,
  purpleDark
)

const safeColors: ColorSet = mergeColors(lightColors, darkColors)

export type CSSVariableMap = Record<string, string>

const generateCSSVariables = (colorSet: ColorSet, withCSSVar?: boolean): CSSVariableMap => {
  return Object.keys(colorSet).reduce((acc, key) => {
    if (withCSSVar) acc[`--colors-${key}` as string] = colorSet[key]
    else {
      const cssVariableName = `--colors-${key}`
      acc[key] = `var(${cssVariableName}, ${colorSet[key]})`
    }

    return acc
  }, {} as CSSVariableMap)
}

export const colors: CSSVariableMap = generateCSSVariables(safeColors)

export const lightColorsVars: CSSVariableMap = {
  ...generateCSSVariables(lightColors, true),
  ...generateCSSVariables(darkColors, true),
}

export const darkColorsVars: CSSVariableMap = {
  ...Object.keys(lightColors).reduce((acc, key) => {
    const name = key.split(/(\d+)/)[0]
    acc[`--colors-${key}` as string] = darkColors[key.replace(name, `${name}Dark`)]
    return acc
  }, {} as CSSVariableMap),
  ...Object.keys(darkColors).reduce((acc, key) => {
    const name = key.split(/(\d+)/)[0].replace('Dark', '')
    acc[`--colors-${key}` as string] = lightColors[key.replace(`${name}Dark`, name)]
    return acc
  }, {} as CSSVariableMap),
}
