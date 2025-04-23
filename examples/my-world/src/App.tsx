import { useEffect } from "react";
import { MapView } from "./components";
import { useUserStore } from "./stores";

function App() {
  const { profiles, createProfile, activeProfileId, setActiveProfile } =
    useUserStore();

  // Create a default profile if none exists
  useEffect(() => {
    if (profiles.length === 0) {
      // Create a default profile
      const newProfile = createProfile("Default User");
      setActiveProfile(newProfile.id);
    } else if (!activeProfileId && profiles.length > 0) {
      // If there are profiles but no active one, set the first one as active
      setActiveProfile(profiles[0].id);
    }
  }, [profiles, activeProfileId]);

  return (
    <div className="h-screen w-screen overflow-hidden">
      <MapView />
    </div>
  );
}

export default App;
