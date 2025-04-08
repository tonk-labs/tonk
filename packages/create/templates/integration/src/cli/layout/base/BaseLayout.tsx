import React from "react";
import { Box } from "ink";

import { Selection, SelectionZone } from "../../components/SelectionZone";

import { Footer } from "./Footer";
import { Header } from "./Header";
import { MainMenu } from "./MainMenu";

interface Props {
  children: React.ReactNode;
}

const activeBoxProps = { borderStyle: "double", borderColor: "cyan" };

export const BaseLayout: React.FC<Props> = ({ children }) => (
  <Box flexDirection="column">
    <Header />
    <Box flexDirection="row" alignSelf="center" minHeight={20}>
      <SelectionZone nextKey="tab" isActive looped>
        <Selection activeProps={activeBoxProps}>
          <Box
            width="20%"
            flexDirection="column"
            paddingX={1}
            borderStyle="single"
          >
            <MainMenu />
          </Box>
        </Selection>

        <Selection activeProps={activeBoxProps}>
          <Box
            width="60%"
            flexDirection="column"
            paddingX={1}
            borderStyle="single"
          >
            {children}
          </Box>
        </Selection>
      </SelectionZone>
    </Box>
    <Footer />
  </Box>
);
