import * as React from "react"

interface DropdownMenuContextValue {
  openDropdownId: string | null
  setOpenDropdownId: (id: string | null) => void
}

const DropdownMenuContext = React.createContext<DropdownMenuContextValue | undefined>(undefined)

export function DropdownMenuProvider({ children }: { children: React.ReactNode }) {
  const [openDropdownId, setOpenDropdownId] = React.useState<string | null>(null)

  const value = React.useMemo(
    () => ({ openDropdownId, setOpenDropdownId }),
    [openDropdownId]
  )

  return (
    <DropdownMenuContext.Provider value={value}>
      {children}
    </DropdownMenuContext.Provider>
  )
}

export function useDropdownMenuContext() {
  const context = React.useContext(DropdownMenuContext)
  if (!context) {
    throw new Error("useDropdownMenuContext must be used within DropdownMenuProvider")
  }
  return context
}

export function useDropdownMenuState(id: string) {
  const { openDropdownId, setOpenDropdownId } = useDropdownMenuContext()

  const isOpen = openDropdownId === id

  const setIsOpen = React.useCallback(
    (open: boolean) => {
      setOpenDropdownId(open ? id : null)
    },
    [id, setOpenDropdownId]
  )

  return { isOpen, setIsOpen }
}
