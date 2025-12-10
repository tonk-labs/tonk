import { useRef, useState, useCallback, useEffect } from 'react';
import { useDockActions } from '../hooks/useDockActions';
import { useMembersBar } from '@/features/members-bar/stores/membersBarStore';
import { useFeatureFlag } from '@/hooks/useFeatureFlag';
import tinkiIcon from '@/assets/images/tinki-icon.png';

interface DockItem {
  id: string;
  label: string;
  icon: string;
  onClick: () => void;
}

const MEMBERS_BAR_WIDTH = 280;

export function Dock() {
  const dockRef = useRef<HTMLDivElement>(null);
  const [mouseX, setMouseX] = useState<number | null>(null);
  const { createNewNote } = useDockActions();

  const showMembersBar = useFeatureFlag('showMembersBar');
  const { isOpen: isMembersBarOpen } = useMembersBar();
  const isMembersBarVisible = showMembersBar && isMembersBarOpen;

  const dockItems: DockItem[] = [
    {
      id: 'tinki',
      label: 'Tinki',
      icon: tinkiIcon,
      onClick: createNewNote,
    },
  ];

  const handleMouseMove = useCallback((e: MouseEvent) => {
    if (!dockRef.current) return;
    const rect = dockRef.current.getBoundingClientRect();

    // Check if mouse is near the dock (within 100px vertically)
    if (e.clientY >= rect.top - 100 && e.clientY <= rect.bottom + 20) {
      setMouseX(e.clientX);
    } else {
      setMouseX(null);
    }
  }, []);

  const handleMouseLeave = useCallback(() => {
    setMouseX(null);
  }, []);

  useEffect(() => {
    document.addEventListener('mousemove', handleMouseMove);
    return () => {
      document.removeEventListener('mousemove', handleMouseMove);
    };
  }, [handleMouseMove]);

  const getScale = (itemIndex: number): number => {
    if (mouseX === null || !dockRef.current) return 1;

    const items = dockRef.current.querySelectorAll('[data-dock-item]');
    const item = items[itemIndex] as HTMLElement;
    if (!item) return 1;

    const rect = item.getBoundingClientRect();
    const itemCenterX = rect.left + rect.width / 2;
    const distance = Math.abs(mouseX - itemCenterX);

    // Max effect radius in pixels
    const maxDistance = 150;

    if (distance > maxDistance) return 1;

    // Calculate scale: closer = bigger (max 1.5x)
    const scale = 1 + (1 - distance / maxDistance) * 0.5;
    return Math.min(scale, 1.5);
  };

  return (
    <div
      ref={dockRef}
      className="fixed bottom-4 mx-auto flex w-fit items-end justify-center gap-1 rounded-xl border bg-white/20 border-gray-200/20 dark:border-[#313138]/20 dark:bg-[#1A1A1E]/20 backdrop-blur-xl px-3 py-2 z-[9999]"
      style={{
        left: 0,
        right: isMembersBarVisible ? MEMBERS_BAR_WIDTH : 0,
      }}
      onMouseLeave={handleMouseLeave}
    >
      {dockItems.map((item, index) => {
        const scale = getScale(index);
        return (
          <div
            key={item.id}
            data-dock-item
            className="flex cursor-pointer flex-col items-center gap-1 p-2 origin-bottom transition-transform duration-150 ease-out group"
            onClick={item.onClick}
            style={{ transform: `scale(${scale})` }}
          >
            <img
              src={item.icon}
              alt={item.label}
              className="h-12 w-12 object-contain transition-transform duration-150 ease-out"
            />
            <span className="whitespace-nowrap text-[11px] opacity-0 group-hover:opacity-100 group-hover:text-black transition-opacity duration-150 ease-out bg-">
              {item.label}
            </span>
          </div>
        );
      })}
    </div>
  );
}
