import { type ChangeEvent, cloneElement, type ReactElement, useRef } from 'react';

interface SelectFileTriggerProps {
  onSelect: (e: ChangeEvent<HTMLInputElement>) => void;
  accept?: string;
  multiple?: boolean;
  disabled?: boolean;
  children: ReactElement<{ onClick?: () => void }>;
}

export function SelectFileTrigger({
  onSelect,
  accept,
  multiple,
  disabled,
  children,
}: SelectFileTriggerProps) {
  const inputRef = useRef<HTMLInputElement>(null);

  const handleClick = () => {
    if (disabled) return;
    // Call the child's existing onClick if it exists
    children.props.onClick?.();

    if (inputRef.current) {
      inputRef.current.value = ''; // Reset so same file triggers change
      inputRef.current.click();
    }
  };

  return (
    <>
      <input
        type="file"
        ref={inputRef}
        className="hidden"
        accept={accept}
        multiple={multiple}
        onChange={onSelect}
        disabled={disabled}
      />
      {cloneElement(children, { onClick: handleClick })}
    </>
  );
}
