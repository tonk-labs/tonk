import {
  DefaultMainMenu,
  DefaultMainMenuContent,
  TldrawUiMenuCheckboxItem,
  TldrawUiMenuGroup,
  TldrawUiMenuItem,
  TldrawUiMenuSubmenu,
} from 'tldraw';
import { useMembersBar } from '@/features/members-bar';
import { useFeatureFlagContext } from '../../../contexts/FeatureFlagContext';

export function FeatureFlagMenu() {
  const { flags, setFlag } = useFeatureFlagContext();
  const showMembersBar = flags.showMembersBar;
  const { isOpen, toggle } = useMembersBar();

  // Only show in development
  if (import.meta.env.PROD) {
    return <DefaultMainMenu />;
  }

  return (
    <DefaultMainMenu>
      <DefaultMainMenuContent />

      {/* Feature Flags submenu */}
      <TldrawUiMenuGroup id="feature-flags">
        <TldrawUiMenuSubmenu id="feature-flags-submenu" label="Feature Flags">
          <TldrawUiMenuGroup id="feature-flags-list">
            {Object.entries(flags).map(([key, value]) => (
              <TldrawUiMenuCheckboxItem
                key={key}
                id={`feature-flag-${key}`}
                label={key}
                checked={value}
                onSelect={() => {
                  setFlag(key as keyof typeof flags, !value);
                }}
              />
            ))}
          </TldrawUiMenuGroup>
        </TldrawUiMenuSubmenu>
      </TldrawUiMenuGroup>

      {/* Members Bar toggle */}
      {showMembersBar && (
        <TldrawUiMenuGroup id="members-bar-toggle">
          <TldrawUiMenuItem
            id="toggle-members-bar"
            label={isOpen ? 'Hide Members Bar' : 'Show Members Bar'}
            onSelect={toggle}
          />
        </TldrawUiMenuGroup>
      )}
    </DefaultMainMenu>
  );
}
