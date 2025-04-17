import React, { useState, useEffect } from "react";
import { useUserStore } from "../stores";

const UserProfile: React.FC = () => {
  const { profiles, activeProfileId, updateProfileName } = useUserStore();
  const [isEditing, setIsEditing] = useState(false);

  // Find the active profile from the profiles array
  const activeProfile = profiles.find(
    (profile) => profile.id === activeProfileId,
  ) || { id: "", name: "Guest" }; // Fallback if no active profile

  const [name, setName] = useState(activeProfile.name);

  // Update name state when active profile changes
  useEffect(() => {
    setName(activeProfile.name);
  }, [activeProfile]);

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    if (name.trim() === "") return;

    updateProfileName(activeProfile.id, name);
    setIsEditing(false);
  };

  return (
    <div className="p-4 bg-white rounded-lg shadow-sm">
      <h2 className="text-xl font-semibold mb-4">Profile</h2>

      {isEditing ? (
        <form onSubmit={handleSubmit} className="flex flex-col gap-3">
          <input
            type="text"
            value={name}
            onChange={(e) => setName(e.target.value)}
            className="px-3 py-2 border rounded"
            placeholder="Your name"
            required
          />
          <div className="flex gap-2">
            <button
              type="submit"
              className="bg-green-500 text-white py-2 px-4 rounded hover:bg-green-600"
            >
              Save
            </button>
            <button
              type="button"
              onClick={() => {
                setIsEditing(false);
                setName(activeProfile.name);
              }}
              className="bg-gray-200 text-gray-800 py-2 px-4 rounded hover:bg-gray-300"
            >
              Cancel
            </button>
          </div>
        </form>
      ) : (
        <div>
          <div className="mb-3">
            <span className="font-medium">Name: </span>
            <span>{activeProfile.name}</span>
          </div>
          <div className="mb-4">
            <span className="font-medium">User ID: </span>
            <span className="text-sm text-gray-600">{activeProfile.id}</span>
          </div>
          <button
            onClick={() => setIsEditing(true)}
            className="bg-blue-500 text-white py-2 px-4 rounded hover:bg-blue-600"
          >
            Edit Profile
          </button>
        </div>
      )}
    </div>
  );
};

export default UserProfile;
