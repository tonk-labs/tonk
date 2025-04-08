import { useRef } from "react";
import { Key, useInput as useInputInk } from "ink";

export type UserInput = { key: Key; raw: string };

export const useInput = (
  handler: (input: UserInput) => void,
  isActive = true
) => {
  const prevented = useRef(false);

  // const { stdin } = useStdin()
  // stdin?.on('data', (data: Buffer) => {
  //   const input = String(data)
  //   // console.log('Pressed', input)
  //   // console.log('Pressed', input === '\u001bOP')
  //   // console.log('Pressed', input === '\u0008')
  // })

  useInputInk(
    (raw, key) => {
      if (prevented.current) {
        prevented.current = false;
        return;
      }

      handler({ key, raw });
    },
    { isActive }
  );

  const prevent = () => {
    prevented.current = true;
  };

  return prevent;
};
