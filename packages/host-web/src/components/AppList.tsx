import { useTonk } from '../context/TonkContext';
import { useDialogs } from '../context/DialogContext';
import type { App } from '../types/index';

export function AppList() {
  const { availableApps, selectedAppIndex, setSelectedAppIndex } = useTonk();
  const { openConfirmationDialog } = useDialogs();

  const handleAppClick = (index: number) => {
    if (index >= 0 && index < availableApps.length) {
      setSelectedAppIndex(index);
      const app = availableApps[index];
      const appName = typeof app === 'string' ? app : app.name;
      openConfirmationDialog(appName);
    }
  };

  if (availableApps.length === 0) {
    return (
      <ul class="list-none">
        <li class="px-3 py-2 text-[#666]">No applications found</li>
      </ul>
    );
  }

  return (
    <ul class="list-none">
      {availableApps.map((app, index) => {
        const appName = typeof app === 'string' ? app : app.name;
        const appStatus = typeof app === 'string' ? 'Ready' : app.status || 'Ready';
        const isSelected = index === selectedAppIndex;

        return (
          <li
            key={appName}
            onClick={() => handleAppClick(index)}
            class={`px-3 py-2 cursor-pointer flex items-center text-white font-semibold transition-colors ${
              isSelected
                ? 'bg-[#ddd] text-black'
                : 'bg-transparent hover:bg-[#333]'
            }`}
          >
            <span>{appName}</span>
            <span
              class={`ml-auto text-xs font-semibold ${
                isSelected ? 'text-black' : 'text-white'
              }`}
            >
              {appStatus}
            </span>
          </li>
        );
      })}
    </ul>
  );
}
