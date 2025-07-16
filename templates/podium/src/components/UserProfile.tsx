import React, { useState } from "react";
import { useLocalAuthStore } from "../stores/userStore";

export const UserProfile: React.FC = () => {
  const { currentUser, updateCurrentUserProfile, logoutUser } =
    useLocalAuthStore();
  const [isEditing, setIsEditing] = useState(false);
  const [formData, setFormData] = useState({
    name: currentUser?.name || "",
    relationToOwner: currentUser?.relationToOwner || "",
    profilePicture: currentUser?.profilePicture || "",
  });

  if (!currentUser) {
    return null;
  }

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    updateCurrentUserProfile(formData);
    setIsEditing(false);
  };

  const handleImageUpload = (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    if (file) {
      const reader = new FileReader();
      reader.onload = (event) => {
        const result = event.target?.result as string;
        setFormData((prev) => ({ ...prev, profilePicture: result }));
      };
      reader.readAsDataURL(file);
    }
  };

  const handleLogout = () => {
    if (window.confirm("Are you sure you want to logout?")) {
      logoutUser();
    }
  };

  return (
    <div className="bg-white rounded-lg shadow p-4 mb-6">
      <div className="flex items-center justify-between mb-4">
        <h3 className="text-lg font-semibold">Your Profile</h3>
        <div className="flex space-x-2">
          <button
            onClick={() => setIsEditing(!isEditing)}
            className="text-sm text-blue-600 hover:text-blue-800"
          >
            {isEditing ? "Cancel" : "Edit"}
          </button>
          <button
            onClick={handleLogout}
            className="text-sm text-orange-600 hover:text-orange-800"
          >
            Logout
          </button>
        </div>
      </div>

      {!isEditing ? (
        <div className="flex items-center space-x-4">
          {currentUser.profilePicture && (
            <img
              src={currentUser.profilePicture}
              alt={currentUser.name}
              className="w-16 h-16 rounded-full object-cover"
            />
          )}
          <div className="flex-1">
            <h4 className="font-semibold text-lg">{currentUser.name}</h4>
            <p className="text-gray-600">
              {currentUser.isOwner ? "Owner" : currentUser.relationToOwner}
            </p>
            {currentUser.isOwner && (
              <div className="mt-2 p-3 bg-blue-50 rounded text-sm">
                <p className="text-blue-800 font-medium mb-1">
                  👑 You're the owner of this Podium!
                </p>
                <p className="text-blue-700 text-xs">
                  Family members can join by visiting this app and creating
                  their own accounts. They'll automatically be added to your
                  family circle.
                </p>
              </div>
            )}
          </div>
        </div>
      ) : (
        <form onSubmit={handleSubmit} className="space-y-4">
          <div>
            <label className="block text-sm font-medium text-gray-700 mb-1">
              Name
            </label>
            <input
              type="text"
              value={formData.name}
              onChange={(e) =>
                setFormData((prev) => ({ ...prev, name: e.target.value }))
              }
              className="w-full px-3 py-2 border border-gray-300 rounded-md focus:outline-none focus:ring-2 focus:ring-blue-500"
              required
            />
          </div>

          {!currentUser.isOwner && (
            <div>
              <label className="block text-sm font-medium text-gray-700 mb-1">
                Relation to Owner
              </label>
              <input
                type="text"
                value={formData.relationToOwner}
                onChange={(e) =>
                  setFormData((prev) => ({
                    ...prev,
                    relationToOwner: e.target.value,
                  }))
                }
                placeholder="e.g., Friend, Family, Cousin, etc."
                className="w-full px-3 py-2 border border-gray-300 rounded-md focus:outline-none focus:ring-2 focus:ring-blue-500"
              />
            </div>
          )}

          <div>
            <label className="block text-sm font-medium text-gray-700 mb-1">
              Profile Picture
            </label>
            <input
              type="file"
              accept="image/*"
              onChange={handleImageUpload}
              className="w-full px-3 py-2 border border-gray-300 rounded-md focus:outline-none focus:ring-2 focus:ring-blue-500"
            />
            {formData.profilePicture && (
              <img
                src={formData.profilePicture}
                alt="Preview"
                className="mt-2 w-16 h-16 rounded-full object-cover"
              />
            )}
          </div>

          <div className="flex space-x-3">
            <button
              type="submit"
              className="flex-1 bg-blue-500 text-white py-2 px-4 rounded-md hover:bg-blue-600 transition-colors"
            >
              Save Changes
            </button>
            <button
              type="button"
              onClick={() => {
                setIsEditing(false);
                setFormData({
                  name: currentUser.name,
                  relationToOwner: currentUser.relationToOwner,
                  profilePicture: currentUser.profilePicture || "",
                });
              }}
              className="flex-1 bg-gray-500 text-white py-2 px-4 rounded-md hover:bg-gray-600 transition-colors"
            >
              Cancel
            </button>
          </div>
        </form>
      )}
    </div>
  );
};
