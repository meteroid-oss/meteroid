import { FC, Fragment } from 'react'

interface Props {
  defaultValue: Breadcrumb[]
}

export interface Breadcrumb {
  key: string
  label: string
  onClick?: () => void
}

export const BreadcrumbsView: FC<Props> = ({ defaultValue: breadcrumbs }) => {
  return (
    <>
      {breadcrumbs?.length
        ? breadcrumbs.map(breadcrumb => (
            <Fragment key={breadcrumb.key}>
              <span className="text-scale-800 dark:text-scale-700">
                <svg
                  viewBox="0 0 24 24"
                  width="16"
                  height="16"
                  stroke="currentColor"
                  strokeWidth="1"
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  fill="none"
                  shapeRendering="geometricPrecision"
                >
                  <path d="M16 3.549L7.12 20.600"></path>
                </svg>
              </span>

              <a
                onClick={breadcrumb.onClick ?? (() => {})}
                className={`text-gray-1100 block px-2 py-1 text-xs leading-5 focus:bg-gray-100 focus:text-gray-900 focus:outline-none ${
                  breadcrumb.onClick ? 'cursor-pointer hover:text-white-100' : ''
                }`}
              >
                {breadcrumb.label}
              </a>
            </Fragment>
          ))
        : null}
    </>
  )
}
