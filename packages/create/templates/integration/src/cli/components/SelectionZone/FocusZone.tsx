import React from 'react'

interface FocusZoneProps {
  id: string
  children: React.ReactNode | React.ReactNode[]
}

export const FocusZone: React.FC<FocusZoneProps> = ({ children }) => {
  return <>{children}</>
}
