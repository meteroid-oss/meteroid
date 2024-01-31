const fs = require('fs')

const postcss = require('postcss')
const selectorParser = require('postcss-selector-parser')

const FILE_PATH = 'dist/tailwind.css'

const selectorsToRemove = [
  "[type='text']",
  "[type='email']",
  "[type='url']",
  "[type='password']",
  "[type='number']",
  "[type='date']",
  "[type='datetime-local']",
  "[type='month']",
  "[type='search']",
  "[type='tel']",
  "[type='time']",
  "[type='week']",
  '[multiple]',
  'textarea',
  'select',
  "[type='text']:focus",
  "[type='email']:focus",
  "[type='url']:focus",
  "[type='password']:focus",
  "[type='number']:focus",
  "[type='date']:focus",
  "[type='datetime-local']:focus",
  "[type='month']:focus",
  "[type='search']:focus",
  "[type='tel']:focus",
  "[type='time']:focus",
  "[type='week']:focus",
  'button',
  "[type='button']",
  "[type='reset']",
  "[type='submit']",
]

fs.readFile(FILE_PATH, 'utf8', (err, css) => {
  if (err) {
    console.error(err)
    return
  }

  const updatedCSS = removeStyles(css)
  writeUpdatedCSS(updatedCSS)
})

function removeStyles(css) {
  const root = postcss.parse(css)
  const updatedRoot = removeSelectors(root)
  return updatedRoot.toString()
}

function removeSelectors(root) {
  root.walkDecls(decl => {
    const parentRule = decl.parent
    if (parentRule && parentRule.type === 'rule') {
      const selectors = parentRule.selectors.filter(selector => {
        const parsedSelector = selectorParser().astSync(selector)

        return !selectorsToRemove.some(selectorToRemove => {
          const parsedSelectorToRemove = selectorParser().astSync(selectorToRemove)
          return parsedSelector.some(node => node.toString() === parsedSelectorToRemove.toString())
        })
      })

      if (selectors.length === 0) {
        parentRule.remove()
      } else {
        parentRule.selectors = selectors
      }
    }
  })

  return root
}

function writeUpdatedCSS(updatedCSS) {
  fs.writeFile(FILE_PATH, updatedCSS, 'utf8', err => {
    if (err) {
      console.error(err)
      return
    }

    console.log('Styles removed successfully!')
  })
}
