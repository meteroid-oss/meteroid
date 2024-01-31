import { ButtonAlt } from '@ui/components'
import { cn } from '@ui/lib'
import { XCircle } from 'lucide-react'

import { EditableTableCell, EditableTableCellProps } from './EditableTableCell'

export const TierEditTable = ({
  headers,
  headerDescriptions,
  tiers,
  addTier,
  removeTier,
  unremovableIndices,
  maxTiers,
  overrideHasInteracted,
}: {
  headers: string[]
  headerDescriptions?: (string | undefined)[]
  tiers: EditableTableCellProps<any>[][]
  addTier: () => void
  removeTier: (idx: number) => void
  unremovableIndices: number[]
  maxTiers?: number
  overrideHasInteracted?: boolean
}) => {
  return (
    <div className="col-span-5">
      <div className="flex flex-col w-full">
        <div>
          <div className="align-middle inline-block min-w-full ">
            <div className="border-gray-40 rounded-md border">
              <table className="min-w-full rounded-t-md divide-y divide-gray-200">
                <thead>
                  <tr className="bg-gray-10">
                    {headers.map((header, idx) => {
                      return (
                        <th
                          key={header}
                          scope="col"
                          className={cn(
                            'px-3 py-2 text-left text-sm font-medium text-gray-60',
                            idx === headers.length - 1 ? '' : 'border-r'
                          )}
                        >
                          <div className="flex">
                            {header}
                            {headerDescriptions && headerDescriptions[idx] && (
                              // <InfoIconWithTooltip
                              //     className="ml-2"
                              //     tooltipContents={
                              //         <SmallText color="text-white">
                              //             {headerDescriptions[idx]}
                              //         </SmallText>
                              //     }
                              // />
                              <></>
                            )}
                          </div>
                        </th>
                      )
                    })}
                  </tr>
                </thead>
                <tbody className="border-none">
                  {tiers.map((tier, idx) => {
                    return (
                      <tr key={`tier-${idx}`} className="m-0 p-0 relative border-t">
                        {tier.map((column, col_idx) => {
                          return (
                            <EditableTableCell
                              //   overrideHasInteracted={overrideHasInteracted}
                              key={col_idx}
                              {...column}
                            />
                          )
                        })}
                        {!unremovableIndices.includes(idx) ? (
                          <div
                            className="float-right flex absolute mt-2.5 -ml-2 text-gray-500 cursor-pointer z-20"
                            onClick={() => removeTier(idx)}
                          >
                            <XCircle className="h-4 w-4 text-gray-500 bg-white hover:text-accent-3" />
                          </div>
                        ) : null}
                      </tr>
                    )
                  })}
                </tbody>
              </table>
            </div>
          </div>
        </div>
      </div>
      <div className="mt-2">
        {!maxTiers ||
          (tiers.length < maxTiers && (
            <ButtonAlt type="link" onClick={addTier}>
              + Add another tier
            </ButtonAlt>
          ))}
      </div>
    </div>
  )
}
