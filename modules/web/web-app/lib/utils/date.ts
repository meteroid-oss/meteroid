import dayjs from 'dayjs'

export const parseAndFormatDate = (date: string) => {
  return dayjs(date).format('DD/MM/YY')
}

export const parseAndFormatDateOptional = (date?: string) => {
  return date ? parseAndFormatDate(date) : '-'
}
