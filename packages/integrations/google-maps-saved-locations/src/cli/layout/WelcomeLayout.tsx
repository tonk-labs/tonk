import React from 'react'
import { Box } from 'ink'

interface Props {
  children: React.ReactNode
}

export const WelcomeLayout: React.FC<Props> = ({ children }) => {
  return (
    <Box
      flexDirection="column"
      justifyContent="center"
      alignItems="center"
      width="100%"
      minHeight={20}
    >
      {children}
    </Box>
  )
}
