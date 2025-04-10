import React, { cloneElement, isValidElement, useEffect } from 'react'
import { Nullable } from 'tsdef'

import { IZone, useSelectionZone } from './SelectionContext'

type Props<ChildProps> = {
  activeProps?: Partial<ChildProps>
  index?: number
  selectedByDefault?: boolean
  children?:
    | React.ReactNode
    | React.ReactNode[]
    | ((isFocused: boolean, zone: Nullable<IZone>) => JSX.Element)
}

export const Selection = <ChildProps extends Record<string, any>>({
  children,
  index,
  activeProps,
  selectedByDefault,
}: Props<ChildProps>) => {
  const zone = useSelectionZone()

  useEffect(() => {
    if (selectedByDefault && index) {
      zone?.select(index)
    }
  }, [])

  if (zone) {
    const isActive = zone.isActive && zone.selection === index

    if (isActive) {
      if (typeof children === 'function') {
        return children(isActive, zone)
      }

      if (isValidElement(children)) {
        return cloneElement(children, {
          ...children.props,
          ...activeProps,
        })
      }
    }
  }

  if (typeof children === 'function') {
    return children(false, zone)
  }

  return <>{children}</>
}

// WIP
export const selectionable = <Props extends Record<string, any>>(
  Component: React.ComponentType<Props>,
  defaultFocusedProps: Partial<Props> = {},
) => {
  const SelectionableComponent: React.FC<
    Props & { focusedProps?: Partial<Props> }
  > = (props) => {
    return <Component {...props} />
  }

  ;(SelectionableComponent as any).selectionable = true
  ;(SelectionableComponent as any).defaultFocusedProps = defaultFocusedProps

  return SelectionableComponent
}
