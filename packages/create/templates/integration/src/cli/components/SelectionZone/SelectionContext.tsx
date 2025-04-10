import { createContext, useContext } from 'react'
import { Nullable } from 'tsdef'

export interface IZone {
  selection: number
  select: (value: number) => void
  isActive: boolean
}

export const SelectionContext = createContext<Nullable<IZone>>(null)

export const useSelectionZone = () => useContext(SelectionContext)
