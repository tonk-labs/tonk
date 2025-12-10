import { FaMoon, FaSun } from 'react-icons/fa6';
import { IoAddCircleSharp } from 'react-icons/io5';
import { TbLayoutNavbarExpandFilled } from 'react-icons/tb';
import { useTheme } from '@/hooks/useTheme';
import type { Bundle } from '../../launcher/types';
import Account from '../account/account';
import SideBarButton from '../sidebarButton/sidebarButton';
import { SelectFileTrigger } from '../ui/selectFileTrigger';

export function Sidebar({
  bundles,
  handleLaunch,
  handleFileUpload,
  importing,
}: {
  bundles: Bundle[];
  handleLaunch: (bundleId: string) => void;
  handleFileUpload: (event: React.ChangeEvent<HTMLInputElement>) => void;
  importing: boolean;
}) {
  const { isDark, toggleTheme } = useTheme();

  return (
    <aside className="bg-white dark:bg-night-950 border-r border-night-200 dark:border-night-900 flex z-10">
      <div className="flex flex-col p-3.5 pt-2 gap-2">
        {/* Branding */}
        <div className="group font-gestalt font-black items-center justify-center text-center">
          <div className="w-full group-hover:hidden inline-flex">tonk</div>
          <div className="w-full hidden group-hover:inline-flex">
            <TbLayoutNavbarExpandFilled />
          </div>
        </div>

        {/* Account */}
        <Account />

        {/* Bundles */}
        {bundles.map(bundle => (
          <SideBarButton
            alt={bundle.name}
            key={bundle.name}
            image={bundle.icon || undefined}
            onClick={() => handleLaunch(bundle.id)}
          >
            {bundle.name.toUpperCase().slice(0, 2)}
          </SideBarButton>
        ))}

        {/* Import Bundle */}
        <SelectFileTrigger
          onSelect={handleFileUpload}
          accept=".tonk"
          disabled={importing}
        >
          <SideBarButton alt="Add Space">
            <IoAddCircleSharp className="text-xl" />
          </SideBarButton>
        </SelectFileTrigger>

        {/* Spacer */}
        <div className="flex grow" />

        {/* Theme Toggle */}
        <SideBarButton onClick={toggleTheme} className="bg-transparent!">
          {isDark ? (
            <FaSun className="text-sm" />
          ) : (
            <FaMoon className="text-sm" />
          )}
        </SideBarButton>
      </div>
    </aside>
  );
}
