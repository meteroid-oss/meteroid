import styled from 'styled-components'
import { css, keyframes } from 'styled-components'

export const tabular = css`
  font-variant-numeric: tabular-nums;
  letter-spacing: -0.4px;
  padding-right: 1px;
`

const Line = styled.div`
  z-index: 1;
  position: absolute;
  height: 100%;
  width: 1px;
  background: hsl(var(--popover) / var(--tw-bg-opacity));
  pointer-events: none;
`

const Tooltip = styled.div`
  z-index: 1;
  position: absolute;
  top: -32px;
  font-size: 14px;
  display: flex;
  align-items: center;
  background: rgba(0, 0, 0, 0.9);
  backdrop-filter: blur(3px);
  box-shadow: 0px 4px 25px rgba(0, 0, 0, 0.25);
  border-radius: 7px;
  padding: 5px 12px;
  white-space: nowrap;
  pointer-events: none;

  span,
  strong {
    ${tabular}
  }
`

const TooltipComparison = styled.div`
  z-index: 1;
  text-align: center;
  background: rgba(0, 0, 0, 0.9);
  backdrop-filter: blur(3px);
  box-shadow: 0px 4px 25px rgba(0, 0, 0, 0.25);
  border-radius: 5px;
  padding: 7px 12px;
`

const Dot = styled.div`
  margin: 0 3px;
  color: hsl(var(--popover) / var(--tw-bg-opacity));
`

const Date = styled.div`
  color: hsl(var(--popover) / var(--tw-bg-opacity));
  ${tabular}
`

const Point = styled.div`
  z-index: 1;
  position: absolute;
  height: 7px;
  width: 7px;
  box-shadow: 0 0 0 3px hsl(var(--popover) / var(--tw-bg-opacity));
  border-radius: 50%;
  background: #fff;
  pointer-events: none;
`

const PointSmall = styled.div`
  z-index: 1;
  position: absolute;
  height: 5px;
  width: 5px;
  border-radius: 50%;
  background: #fff;
  pointer-events: none;
`

const ChartCrosshair = { Line, Tooltip, TooltipComparison, Dot, Date, Point, PointSmall }

export default ChartCrosshair
