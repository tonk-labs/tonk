import { cn } from '@/lib/utils';

interface GradientTextProps {
  children: React.ReactNode;
  className?: string;
}

export function GradientText({ children, className }: GradientTextProps) {
  return (
    <span
      className={cn(
        'animate-shine bg-clip-text text-transparent bg-gradient-to-r from-[#e6c767] via-[#a8d8b9] to-[#e6c767] bg-[length:200%_auto]',
        className
      )}
    >
      {children}
    </span>
  );
}
