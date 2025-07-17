import React, { useState, useEffect } from "react";
import { useSyncedUsersStore, useLocalAuthStore } from "../stores/userStore";
import { PasskeyManager } from "../utils/passkey";
import { UserProfile } from "./UserProfile";

export const UserAuth: React.FC = () => {
  const { users } = useSyncedUsersStore();
  const {
    currentUser,
    isAuthenticated,
    registerUser,
    authenticateWithPasskey,
    tryAutoLogin,
  } = useLocalAuthStore();
  const [isAuthenticating, setIsAuthenticating] = useState(false);
  const [formData, setFormData] = useState({
    name: "",
    relationToOwner: "",
    profilePicture: "",
  });

  useEffect(() => {
    tryAutoLogin();
  }, [tryAutoLogin]);

  const handleRegister = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!formData.name.trim()) return;

    try {
      setIsAuthenticating(true);
      await registerUser(formData);
      setFormData({ name: "", relationToOwner: "", profilePicture: "" });
    } catch (error) {
      console.error("Registration failed:", error);
      alert("Failed to create account. Please try again.");
    } finally {
      setIsAuthenticating(false);
    }
  };

  const handlePasskeyLogin = async () => {
    try {
      setIsAuthenticating(true);

      const user = await authenticateWithPasskey();
      if (!user) {
        alert(
          "Authentication failed. Please try again or create a new account.",
        );
      }
    } catch (error) {
      console.error("Passkey authentication failed:", error);
      alert("Authentication failed. Please try again.");
    } finally {
      setIsAuthenticating(false);
    }
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

  if (isAuthenticated && currentUser) {
    return <UserProfile />;
  }

  const hasStoredPasskey = PasskeyManager.hasStoredPasskey();
  const isPasskeySupported = PasskeyManager.isSupported();

  return (
    <div className="card">
      <h2>
        {users.length === 0
          ? "Welcome! Let's set up your Podium."
          : "Join the Family"}
      </h2>

      {/* Show different UI based on whether user has a stored passkey */}
      {hasStoredPasskey ? (
        <div style={{ textAlign: "center" }}>
          <p style={{ marginBottom: "1.5rem" }}>
            Welcome back! Use your passkey to sign in.
          </p>
          <button
            onClick={handlePasskeyLogin}
            disabled={isAuthenticating}
            className="btn btn-primary"
            style={{ width: "100%", marginBottom: "1rem" }}
          >
            {isAuthenticating ? "Authenticating..." : "🔐 Sign in with Passkey"}
          </button>
          <p style={{ fontSize: "0.8rem", opacity: 0.6 }}>
            You'll be prompted to use your fingerprint, face, or device PIN
          </p>
        </div>
      ) : (
        <div>
          {/* Simple create account form */}
          <form onSubmit={handleRegister}>
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
                      disabled={isAuthenticating}
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
                    placeholder="Enter your name"
                    className="form-input"
                    required
                    disabled={isAuthenticating}
                  />
                </div>
              </div>
            </div>

            {users.length > 0 && (
              <div className="form-group">
                <label className="form-label">Relation to Owner</label>
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
                  className="form-input"
                  disabled={isAuthenticating}
                />
              </div>
            )}

            <button
              type="submit"
              disabled={isAuthenticating}
              className="btn btn-primary"
              style={{ width: "100%" }}
            >
              {isAuthenticating
                ? "Creating Account..."
                : users.length === 0
                  ? "Create Account & Become Owner"
                  : "Join Family"}
            </button>
          </form>

          {isPasskeySupported && (
            <div style={{ textAlign: "center", marginTop: "1rem" }}>
              <p style={{ fontSize: "0.8rem", opacity: 0.6 }}>
                {users.length === 0
                  ? "A secure passkey will be created using your device's biometrics"
                  : "You'll be prompted to use your fingerprint, face, or device PIN"}
              </p>
            </div>
          )}

          {!isPasskeySupported && (
            <div style={{ textAlign: "center", marginTop: "1rem" }}>
              <p
                style={{
                  fontSize: "0.8rem",
                  padding: "0.5rem",
                  background: "rgba(255, 193, 7, 0.1)",
                  borderRadius: "6px",
                  opacity: 0.8,
                }}
              >
                Passkeys not supported. A fallback ID will be used.
              </p>
            </div>
          )}

          {/* Existing user login */}
          {users.length > 0 && (
            <div
              style={{
                textAlign: "center",
                marginTop: "1.5rem",
                paddingTop: "1.5rem",
                borderTop: "1px solid rgba(0, 0, 0, 0.05)",
              }}
            >
              <p
                style={{
                  fontSize: "0.9rem",
                  marginBottom: "0.5rem",
                  opacity: 0.7,
                }}
              >
                Already have an account?
              </p>
              <button
                onClick={handlePasskeyLogin}
                disabled={isAuthenticating}
                className="btn btn-secondary"
                style={{ fontSize: "0.85rem" }}
              >
                Sign in with your passkey
              </button>
            </div>
          )}
        </div>
      )}
    </div>
  );
};
