import { useTonk } from '../context/TonkContext';
import { useDialogs } from '../context/DialogContext';
import icon_upload from "../assets/icon-upload.svg";

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
      <ul class="list-none flex items-center justify-center w-full">
        <div class="flex flex-col">
        <img src={icon_upload} alt="upload icon" class="w-16 h-16 mx-auto"/>
        <p>Drag a .tonk file to load it.</p></div>
      </ul>
    );
  }

  return (
    <ul class="list-none">
      {availableApps.map((app, index) => {
        const appName = typeof app === 'string' ? app : app.name;
        const appStatus =
          typeof app === 'string' ? 'Ready' : app.status || 'Ready';
        const isSelected = index === selectedAppIndex;

        return (
          <li
            key={appName}
            onClick={() => handleAppClick(index)}
            class={`px-3 py-2 cursor-pointer flex items-center text-white font-semibold transition-colors ${
              isSelected
                ? 'bg-[#ddd]/30 text-black'
                : 'bg-transparent hover:bg-[#333]'
            }`}
          >
            <span>{appName}</span>
            <span class="pl-4 text-xs">{appStatus}</span>
          </li>
        );
      })}
    </ul>
  );
}
