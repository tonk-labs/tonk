import { Button } from '@base-ui-components/react';
import { Avatar } from '@base-ui-components/react/avatar';
import { cn } from '@/lib/utils';

type SidebarButtonProps = {
  alt?: string;
  image?: string | undefined;
  onClick?: () => void;
  className?: string;
  children?: React.ReactNode;
  isSelected?: boolean;
};

export default function SideBarButton({
  alt,
  children,
  image = undefined,
  onClick,
  className,
  isSelected = false,
}: SidebarButtonProps) {
  return (
    <Avatar.Root
      className={cn(
        'relative bg-night-100 dark:bg-night-900 cursor-pointer inline-flex justify-center items-center rounded-full hover:scale-105 active:scale-98 transition-all duration-50 w-10 h-10 overflow-hidden text-base leading-1 text-night-950 dark:text-night-100 font-medium select-none align-middle',
        isSelected &&
          'ring-2 ring-night-950 dark:ring-night-100 ring-offset-2 ring-offset-white dark:ring-offset-night-950',
        className
      )}
      render={
        <Button aria-label={alt} title={alt} onClick={onClick} className="group flex gap-8" />
      }
    >
      <Avatar.Image
        src={image ? image : undefined}
        width="48"
        height="48"
        className="bg-cover h-full w-full"
      />
      <Avatar.Fallback className="items-center justify-center flex w-full h-full text-base">
        {children}
      </Avatar.Fallback>
    </Avatar.Root>
  );
}
