import { useEffect, useState } from "react";
import { Key } from "ink";

import { clamp } from "../utils/clamp";

import { useInput, UserInput } from "./useInput";

export type SuperKey =
  | `${"meta" | "ctrl"}+${keyof Key | string}`
  | keyof Key
  | "any";

export const checkSuperKey = (
  input: UserInput,
  superKey: SuperKey | SuperKey[]
) => {
  const superKeys = Array.isArray(superKey) ? superKey : [superKey];

  return superKeys.some((superKey) => {
    if (superKey === "any") {
      return true;
    }

    const keys = superKey.split("+") as (keyof Key)[];
    return keys.every((k) => input.key[k] || input.raw === k);
  });
};

export interface SelectionSettings {
  amount?: number;
  defaultSelection?: number;
  nextKey: SuperKey | SuperKey[];
  prevKey?: SuperKey | SuperKey[];
  isActive?: boolean;
  looped?: boolean;
  onChange?: (selection: number, prevSelection: number) => void;
}

export const useSelection = ({
  amount: _amount = 0,
  defaultSelection = 0,
  prevKey,
  nextKey,
  isActive,
  looped = false,
  onChange,
}: SelectionSettings) => {
  const [selection, setSelection] = useState(defaultSelection);
  const [amount, setAmount] = useState(_amount);
  const maxSelection = amount - 1;

  useEffect(() => {
    if (amount !== 0 && selection > maxSelection) {
      setSelection(maxSelection);
    }
  }, [amount]);

  const prevent = useInput((input) => {
    if (checkSuperKey(input, nextKey)) {
      setSelection((i) => {
        const newSelection = clamp(i + 1, 0, maxSelection, looped);

        onChange && onChange(newSelection, i);

        return newSelection;
      });
    }

    if (prevKey && checkSuperKey(input, prevKey)) {
      setSelection((i) => {
        const newSelection = clamp(i - 1, 0, maxSelection, looped);

        onChange && onChange(newSelection, i);

        return newSelection;
      });
    }
  }, isActive);

  return {
    selection,
    select: setSelection,
    prevent,
    isActive,
    setAmount,
  };
};
