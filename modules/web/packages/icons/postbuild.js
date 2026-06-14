// replace-in-file v8 is ESM-only and exposes named exports ({ replaceInFile, ... })
// with no default callable, whereas v7 exports the function directly. Support both.
const rif = require('replace-in-file')
const replaceInFile = rif.replaceInFile || rif

const options = {
  files: ['./dist/**/*.tsx'],
  from: /(fill|stroke)="#([\w\d]+)"/g,
  to: "$1={props.$1 || 'currentColor'}",
}

replaceInFile(options)
  .then(() => console.log('Icons successfully generated!'))
  .catch(error => console.error('Error occurred:', error))
