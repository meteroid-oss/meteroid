const replace = require('replace-in-file')

const options = {
  files: ['./dist/**/*.tsx'],
  from: /(fill|stroke)="#([\w\d]+)"/g,
  to: "$1={props.$1 || 'currentColor'}",
}

replace(options, error => {
  if (error) {
    return console.error('Error occurred:', error)
  }
  console.log('Icons successfully generated!')
})
