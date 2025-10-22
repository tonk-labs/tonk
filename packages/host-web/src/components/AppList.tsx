import { useTonk } from '../context/TonkContext';
import { useDialogs } from '../context/DialogContext';
import icon_upload from '../assets/icon-upload.svg';
import { useEffect } from 'preact/hooks';
import icon_file from '../assets/icon-tonk.svg';
import { useServiceWorker } from '../hooks/useServiceWorker';

export function AppList() {
  const { availableApps, selectedAppIndex, setSelectedAppIndex } = useTonk();
  const { openConfirmationDialog } = useDialogs();
  const { confirmBoot } = useServiceWorker();

  const handleAppClick = (index: number) => {
    if (index >= 0 && index < availableApps.length) {
      setSelectedAppIndex(index);
      const app = availableApps[index];
      const appName = typeof app === 'string' ? app : app.name;
      openConfirmationDialog(appName);
    }
  };
  const bootTheApp = async (index: number) => {
    await confirmBoot(availableApps[0].name);
  };

  if (availableApps.length === 0) {
    return (
      <ul class="list-none flex items-center justify-center w-full">
        <div class="flex flex-col">
          <img src={icon_upload} alt="upload icon" class="w-16 h-16 mx-auto" />
          <p>Drag a .tonk file to load it.</p>
        </div>
      </ul>
    );
  }

  useEffect(() => {
    if (availableApps.length > 0) {
      bootTheApp(0);
    }
  }, [availableApps.length]);

  return (
    <ul class="list-none flex items-center justify-center w-full">
      {availableApps.map((app, index) => {
        const appName = typeof app === 'string' ? app : app.name;
        const appStatus =
          typeof app === 'string' ? 'Ready' : app.status || 'Ready';
        const isSelected = index === selectedAppIndex;

        return (
          <li
            key={appName}
            onClick={() => bootTheApp(index)}
            class={`border border-black px-3 py-2 cursor-pointer flex items-center text-black font-semibold transition-colors ${
              isSelected
                ? 'bg-[#ddd]/30 text-black'
                : 'bg-transparent hover:bg-[#333]'
            }`}
          >
            <span>
              <img src={icon_file} alt="app icon" class="w-6 h-6 mr-2" />
            </span>
            <span class="pl-4 text-[6pt]">{appStatus}</span>
          </li>
        );
      })}
    </ul>
  );
}
