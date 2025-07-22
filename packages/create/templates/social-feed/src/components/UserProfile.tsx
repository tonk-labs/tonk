import React, { useState } from "react";
import { useLocalAuthStore } from "../stores/userStore";

export const UserProfile: React.FC = () => {
  const { currentUser, updateCurrentUserProfile, logoutUser } =
    useLocalAuthStore();
  const [isEditing, setIsEditing] = useState(false);
  const [formData, setFormData] = useState({
    name: currentUser?.name || "",
    profilePicture: currentUser?.profilePicture || "",
  });

  if (!currentUser) {
    return null;
  }

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    updateCurrentUserProfile({ ...formData, relationToOwner: "" });
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
    <div className="card">
      <div
        style={{
          display: "flex",
          alignItems: "center",
          justifyContent: "space-between",
          marginBottom: "1rem",
        }}
      >
        <h3>Your Profile</h3>
        <div style={{ display: "flex", gap: "0.5rem" }}>
          <button
            onClick={() => setIsEditing(!isEditing)}
            className="btn btn-secondary"
            style={{ fontSize: "0.8rem", padding: "0.25rem 0.75rem" }}
          >
            {isEditing ? "Cancel" : "Edit"}
          </button>
          <button
            onClick={handleLogout}
            className="btn btn-danger"
            style={{ fontSize: "0.8rem", padding: "0.25rem 0.75rem" }}
          >
            Logout
          </button>
        </div>
      </div>

      {!isEditing ? (
        <div style={{ display: "flex", alignItems: "flex-start", gap: "1rem" }}>
          {currentUser.profilePicture && (
            <img
              src={currentUser.profilePicture}
              alt={currentUser.name}
              style={{
                width: "64px",
                height: "64px",
                borderRadius: "50%",
                objectFit: "cover",
                flexShrink: 0,
              }}
            />
          )}
          <div style={{ flex: 1 }}>
            <h4
              style={{
                fontSize: "1.1rem",
                fontWeight: 500,
                marginBottom: "0.25rem",
              }}
            >
              {currentUser.name}
            </h4>
            <p style={{ opacity: 0.7, fontSize: "0.9rem" }}>Member</p>
          </div>
        </div>
      ) : (
        <form onSubmit={handleSubmit}>
          <div className="form-group">
            <label className="form-label">Your Details</label>
            <div className="name-profile-row">
              <div className="profile-picture-upload">
                <div
                  className={`profile-picture-preview ${formData.profilePicture ? "has-image" : ""}`}
                >
                  <input
                    type="file"
                    accept="image/*"
                    onChange={handleImageUpload}
                    className="profile-picture-input"
                  />
                  {formData.profilePicture ? (
                    <>
                      <img
                        src={formData.profilePicture}
                        alt="Profile preview"
                      />
                      <div className="profile-picture-change-overlay">
                        Change
                      </div>
                    </>
                  ) : (
                    <div className="profile-picture-placeholder">
                      <div className="profile-picture-icon">📷</div>
                      <div className="profile-picture-text">Add Photo</div>
                    </div>
                  )}
                </div>
              </div>
              <div className="name-input-container">
                <input
                  type="text"
                  value={formData.name}
                  onChange={(e) =>
                    setFormData((prev) => ({ ...prev, name: e.target.value }))
                  }
                  className="form-input"
                  required
                />
              </div>
            </div>
          </div>

          <div style={{ display: "flex", gap: "0.75rem" }}>
            <button
              type="submit"
              className="btn btn-primary"
              style={{ flex: 1 }}
            >
              Save Changes
            </button>
            <button
              type="button"
              onClick={() => {
                setIsEditing(false);
                setFormData({
                  name: currentUser.name,
                  profilePicture: currentUser.profilePicture || "",
                });
              }}
              className="btn btn-secondary"
              style={{ flex: 1 }}
            >
              Cancel
            </button>
          </div>
        </form>
      )}
    </div>
  );
};
