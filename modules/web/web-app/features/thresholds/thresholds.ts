import { z } from 'zod'

export const MAX_THRESHOLDS = 10

export const parseThresholds = (text: string): number[] => {
  if (!text.trim()) return []
  const values = text.split(',').map(part => part.trim())
  const numbers = values.map(value => (/^\d+$/.test(value) ? Number(value) : NaN))
  const valid =
    numbers.length <= MAX_THRESHOLDS &&
    numbers.every(
      (value, index) => value >= 1 && value <= 100 && (index === 0 || value > numbers[index - 1])
    )
  if (!valid)
    throw new Error(
      `Enter up to ${MAX_THRESHOLDS} increasing whole percentages between 1 and 100, e.g. 50, 80, 95`
    )
  return numbers
}

export const thresholdsSchema = z.string().superRefine((text, ctx) => {
  try {
    parseThresholds(text)
  } catch (error) {
    ctx.addIssue({ code: 'custom', message: (error as Error).message })
  }
})

export const formatThresholds = (thresholds: readonly number[]): string => thresholds.join(', ')
