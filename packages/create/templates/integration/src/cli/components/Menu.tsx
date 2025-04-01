import React, { useState } from "react";
import { Box } from "ink";

import { SelectionSettings, SuperKey, useKey, useSelection } from "../hooks";

import { Selection, UncontrolledSelectionZone } from "./SelectionZone";
import { TextButton } from "./TextButton";

export interface MenuItem {
  title: React.ReactNode;
  items?: SubMenuItem[];
  onSelect?: () => void;
}

type SubMenuItem = Omit<MenuItem, "items">;

interface MenuProps extends Omit<SelectionSettings, "amount" | "onChange"> {
  selectKey?: SuperKey | SuperKey[];
  expandMenuKey?: SuperKey | SuperKey[];
  items: MenuItem[];
  onChangeSelection?: SelectionSettings["onChange"];
}

interface NestedMenuProps {
  selectKey?: SuperKey | SuperKey[];
  isActive: boolean;
  items: SubMenuItem[];
  onReturn: () => void;
  onChangeSelection?: SelectionSettings["onChange"];
}

const NestedMenu: React.FC<NestedMenuProps> = ({
  isActive,
  items,
  onReturn,
  onChangeSelection,
  selectKey,
}) => {
  useKey(["escape", "leftArrow"], onReturn, isActive);

  return (
    <Box marginLeft={1}>
      <Menu
        items={items}
        isActive={isActive}
        prevKey="upArrow"
        nextKey="downArrow"
        selectKey={selectKey}
        onChangeSelection={onChangeSelection}
      />
    </Box>
  );
};

export const Menu: React.FC<MenuProps> = ({
  isActive = false,
  items,
  prevKey,
  nextKey,
  looped,
  selectKey = "return",
  expandMenuKey = "rightArrow",
  onChangeSelection,
}) => {
  const [nestedMenusVisibility, setNestedMenusVisibility] = useState<
    Record<string, boolean>
  >({});

  const anyNestedMenuIsVisible = Object.values(nestedMenusVisibility).some(
    Boolean
  );

  const {
    selection,
    select,
    setAmount,
    isActive: menuIsActive,
  } = useSelection({
    prevKey,
    nextKey,
    isActive: isActive && !anyNestedMenuIsVisible,
    looped,
    onChange: onChangeSelection,
  });

  const openNestedMenu = (id: string) => {
    setNestedMenusVisibility({
      ...nestedMenusVisibility,
      [id]: true,
    });
  };

  const closeNestedMenu = (id: string) => {
    setNestedMenusVisibility({
      ...nestedMenusVisibility,
      [id]: false,
    });
  };

  const handleChangeSelection = (
    id: string,
    currSelection: number,
    prevSelection: number
  ) => {
    if (currSelection === prevSelection) {
      process.nextTick(() => {
        if (currSelection === 0) {
          select(selection - 1);
        } else {
          select(selection + 1);
        }

        closeNestedMenu(id);
      });
    }
  };

  return (
    <Box flexDirection="column">
      <UncontrolledSelectionZone
        selection={selection}
        select={select}
        onChangeAmount={setAmount}
        isActive={menuIsActive}
      >
        {items.map((item, index) => {
          const id = String(index);
          const isVisible = nestedMenusVisibility[id];

          return (
            <Box key={id} flexDirection="column">
              <Selection>
                {(isFocused) => (
                  <TextButton
                    selectKey={item.items ? expandMenuKey : selectKey}
                    isFocused={isFocused || isVisible}
                    onPress={
                      item.items
                        ? () => openNestedMenu(id)
                        : item.onSelect || (() => null)
                    }
                  >
                    {item.title} {item.items && (isVisible ? "▼" : "▶")}
                  </TextButton>
                )}
              </Selection>
              {item.items && isVisible && (
                <NestedMenu
                  onReturn={() => closeNestedMenu(id)}
                  isActive={isActive}
                  items={item.items}
                  onChangeSelection={(selection, prevSelection) =>
                    handleChangeSelection(id, selection, prevSelection)
                  }
                  selectKey={selectKey}
                />
              )}
            </Box>
          );
        })}
      </UncontrolledSelectionZone>
    </Box>
  );
};
