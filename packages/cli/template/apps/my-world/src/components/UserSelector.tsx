import React, { useState } from "react";
import { useUserStore } from "../stores";
import { User, Plus, Edit2, Trash2 } from "lucide-react";

const UserSelector: React.FC = () => {
  const {
    profiles,
    activeProfileId,
    createProfile,
    setActiveProfile,
    updateProfileName,
    deleteProfile,
  } = useUserStore();
  const [isCreatingProfile, setIsCreatingProfile] = useState(false);
  const [isEditingProfile, setIsEditingProfile] = useState(false);
  const [newProfileName, setNewProfileName] = useState("");
  const [editingProfileId, setEditingProfileId] = useState<string | null>(null);

  // Apple design system colors
  const appleColors = {
    blue: "#007AFF",
    green: "#34C759",
    red: "#FF3B30",
    yellow: "#FFCC00",
    gray: {
      light: "#F2F2F7",
      medium: "#E5E5EA",
      dark: "#8E8E93",
    },
    text: {
      primary: "#000000",
      secondary: "#8E8E93",
    },
  };

  const handleCreateProfile = (e: React.FormEvent) => {
    e.preventDefault();
    if (newProfileName.trim() === "") return;

    const newProfile = createProfile(newProfileName);
    setActiveProfile(newProfile.id);
    setNewProfileName("");
    setIsCreatingProfile(false);
  };

  const handleUpdateProfile = (e: React.FormEvent) => {
    e.preventDefault();
    if (!editingProfileId || newProfileName.trim() === "") return;

    updateProfileName(editingProfileId, newProfileName);
    setNewProfileName("");
    setIsEditingProfile(false);
    setEditingProfileId(null);
  };

  const handleDeleteProfile = (id: string) => {
    if (profiles.length <= 1) {
      alert(
        "Cannot delete the last profile. Please create another profile first.",
      );
      return;
    }

    deleteProfile(id);
  };

  const startEditingProfile = (profile: { id: string; name: string }) => {
    setEditingProfileId(profile.id);
    setNewProfileName(profile.name);
    setIsEditingProfile(true);
  };

  return (
    <div
      className="mb-6 rounded-xl overflow-hidden user-selector"
      style={{ backgroundColor: appleColors.gray.light }}
    >
      <div
        className="px-4 py-3 border-b"
        style={{ borderColor: "rgba(0, 0, 0, 0.05)" }}
      >
        <h3
          className="font-semibold flex items-center gap-2"
          style={{ fontSize: "15px" }}
        >
          <User className="h-4 w-4" />
          User Profiles
        </h3>
      </div>

      <div className="p-4">
        {/* Profile Creation Form */}
        {isCreatingProfile ? (
          <form onSubmit={handleCreateProfile} className="mb-4">
            <div className="mb-3">
              <label
                htmlFor="new-profile-name"
                className="text-sm font-medium"
                style={{ color: appleColors.text.secondary }}
              >
                New Profile Name
              </label>
              <input
                id="new-profile-name"
                type="text"
                value={newProfileName}
                onChange={(e) => setNewProfileName(e.target.value)}
                className="w-full px-3 py-2 border mt-1 focus:outline-none focus:ring-2"
                style={{
                  borderColor: appleColors.gray.medium,
                  borderRadius: "8px",
                  fontSize: "15px",
                }}
                placeholder="Enter profile name"
                autoFocus
                required
              />
            </div>
            <div className="flex gap-2">
              <button
                type="button"
                onClick={() => {
                  setIsCreatingProfile(false);
                  setNewProfileName("");
                }}
                className="flex-1 py-2 px-3 text-sm rounded-lg font-medium"
                style={{
                  backgroundColor: "rgba(142, 142, 147, 0.12)",
                  color: appleColors.text.primary,
                }}
              >
                Cancel
              </button>
              <button
                type="submit"
                className="flex-1 py-2 px-3 text-sm rounded-lg font-medium text-white"
                style={{ backgroundColor: appleColors.blue }}
              >
                Create
              </button>
            </div>
          </form>
        ) : isEditingProfile ? (
          <form onSubmit={handleUpdateProfile} className="mb-4">
            <div className="mb-3">
              <label
                htmlFor="edit-profile-name"
                className="text-sm font-medium"
                style={{ color: appleColors.text.secondary }}
              >
                Edit Profile Name
              </label>
              <input
                id="edit-profile-name"
                type="text"
                value={newProfileName}
                onChange={(e) => setNewProfileName(e.target.value)}
                className="w-full px-3 py-2 border mt-1 focus:outline-none focus:ring-2"
                style={{
                  borderColor: appleColors.gray.medium,
                  borderRadius: "8px",
                  fontSize: "15px",
                }}
                placeholder="Enter profile name"
                autoFocus
                required
              />
            </div>
            <div className="flex gap-2">
              <button
                type="button"
                onClick={() => {
                  setIsEditingProfile(false);
                  setEditingProfileId(null);
                  setNewProfileName("");
                }}
                className="flex-1 py-2 px-3 text-sm rounded-lg font-medium"
                style={{
                  backgroundColor: "rgba(142, 142, 147, 0.12)",
                  color: appleColors.text.primary,
                }}
              >
                Cancel
              </button>
              <button
                type="submit"
                className="flex-1 py-2 px-3 text-sm rounded-lg font-medium text-white"
                style={{ backgroundColor: appleColors.blue }}
              >
                Update
              </button>
            </div>
          </form>
        ) : (
          <button
            onClick={() => setIsCreatingProfile(true)}
            className="mb-4 w-full py-2 px-3 text-sm rounded-lg font-medium flex items-center justify-center gap-2"
            style={{
              backgroundColor: appleColors.blue,
              color: "white",
            }}
          >
            <Plus className="h-4 w-4" />
            Create New Profile
          </button>
        )}

        {/* Profile List */}
        <div className="space-y-2 max-h-[200px] overflow-y-auto">
          {profiles.length === 0 ? (
            <div
              className="text-center p-4 text-sm"
              style={{ color: appleColors.text.secondary }}
            >
              No profiles yet. Create your first profile.
            </div>
          ) : (
            profiles.map((profile) => (
              <div
                key={profile.id}
                className={`p-3 rounded-lg flex items-center justify-between ${
                  profile.id === activeProfileId ? "bg-blue-50" : "bg-white"
                }`}
                style={{
                  borderLeft:
                    profile.id === activeProfileId
                      ? `4px solid ${appleColors.blue}`
                      : "none",
                }}
              >
                <div className="flex items-center gap-2">
                  <User
                    className="h-5 w-5"
                    style={{
                      color:
                        profile.id === activeProfileId
                          ? appleColors.blue
                          : appleColors.text.secondary,
                    }}
                  />
                  <div>
                    <div className="font-medium">{profile.name}</div>
                    <div
                      className="text-xs"
                      style={{ color: appleColors.text.secondary }}
                    >
                      {profile.id === activeProfileId ? "Active" : ""}
                    </div>
                  </div>
                </div>
                <div className="flex items-center gap-2">
                  {profile.id !== activeProfileId && (
                    <button
                      onClick={() => setActiveProfile(profile.id)}
                      className="text-xs px-2 py-1 rounded"
                      style={{ color: appleColors.blue }}
                    >
                      Select
                    </button>
                  )}
                  <button
                    onClick={() => startEditingProfile(profile)}
                    className="p-1 rounded-full hover:bg-gray-100"
                    aria-label="Edit profile"
                  >
                    <Edit2
                      className="h-4 w-4"
                      style={{ color: appleColors.text.secondary }}
                    />
                  </button>
                  <button
                    onClick={() => handleDeleteProfile(profile.id)}
                    className="p-1 rounded-full hover:bg-gray-100"
                    aria-label="Delete profile"
                    disabled={profiles.length <= 1}
                  >
                    <Trash2
                      className="h-4 w-4"
                      style={{
                        color:
                          profiles.length <= 1
                            ? appleColors.gray.dark
                            : appleColors.red,
                        opacity: profiles.length <= 1 ? 0.5 : 1,
                      }}
                    />
                  </button>
                </div>
              </div>
            ))
          )}
        </div>
      </div>
    </div>
  );
};

export default UserSelector;
