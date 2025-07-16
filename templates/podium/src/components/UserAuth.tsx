import React, { useState, useEffect } from 'react';
import { useSyncedUsersStore, useLocalAuthStore } from '../stores/userStore';
import { PasskeyManager } from '../utils/passkey';
import { UserProfile } from './UserProfile';

export const UserAuth: React.FC = () => {
  const { users } = useSyncedUsersStore();
  const { currentUser, isAuthenticated, registerUser, authenticateWithPasskey, tryAutoLogin } = useLocalAuthStore();
  const [isAuthenticating, setIsAuthenticating] = useState(false);
  const [formData, setFormData] = useState({
    name: '',
    relationToOwner: '',
    profilePicture: '',
  });

  useEffect(() => {
    tryAutoLogin();
  }, [tryAutoLogin]);

  const handleRegister = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!formData.name.trim()) return;

    try {
      setIsAuthenticating(true);
      const user = await registerUser(formData);
      console.log('User registered:', user);
      setFormData({ name: '', relationToOwner: '', profilePicture: '' });
    } catch (error) {
      console.error('Registration failed:', error);
      alert('Failed to create account. Please try again.');
    } finally {
      setIsAuthenticating(false);
    }
  };

  const handlePasskeyLogin = async () => {
    try {
      setIsAuthenticating(true);
      
      // Debug stored data
      PasskeyManager.debugStoredData();
      
      const user = await authenticateWithPasskey();
      if (!user) {
        alert('Authentication failed. Please try again or create a new account.');
      }
    } catch (error) {
      console.error('Passkey authentication failed:', error);
      alert('Authentication failed. Please try again.');
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
        setFormData(prev => ({ ...prev, profilePicture: result }));
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
    <div className="bg-white rounded-lg shadow p-6 mb-6">
      <h2 className="text-xl font-bold mb-4">
        {users.length === 0 ? 'Welcome! Create Your Account (You\'ll be the owner!)' : 'Join the Family'}
      </h2>

      {/* Show different UI based on whether user has a stored passkey */}
      {hasStoredPasskey ? (
        <div className="text-center space-y-4">
          <p className="text-gray-600">
            Welcome back! Use your passkey to sign in.
          </p>
          <button
            onClick={handlePasskeyLogin}
            disabled={isAuthenticating}
            className="w-full bg-blue-500 text-white py-3 px-4 rounded-md hover:bg-blue-600 transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
          >
            {isAuthenticating ? 'Authenticating...' : '🔐 Sign in with Passkey'}
          </button>
          <p className="text-xs text-gray-500">
            You'll be prompted to use your fingerprint, face, or device PIN
          </p>
        </div>
      ) : (
        <div className="space-y-4">
          {/* Simple create account form */}
          <form onSubmit={handleRegister} className="space-y-4">
            <div>
              <label className="block text-sm font-medium text-gray-700 mb-1">
                Your Name *
              </label>
              <input
                type="text"
                value={formData.name}
                onChange={(e) => setFormData(prev => ({ ...prev, name: e.target.value }))}
                placeholder="Enter your name"
                className="w-full px-3 py-2 border border-gray-300 rounded-md focus:outline-none focus:ring-2 focus:ring-blue-500"
                required
                disabled={isAuthenticating}
              />
            </div>

            {users.length > 0 && (
              <div>
                <label className="block text-sm font-medium text-gray-700 mb-1">
                  Relation to Owner
                </label>
                <input
                  type="text"
                  value={formData.relationToOwner}
                  onChange={(e) => setFormData(prev => ({ ...prev, relationToOwner: e.target.value }))}
                  placeholder="e.g., Friend, Family, Cousin, etc."
                  className="w-full px-3 py-2 border border-gray-300 rounded-md focus:outline-none focus:ring-2 focus:ring-blue-500"
                  disabled={isAuthenticating}
                />
              </div>
            )}

            <div>
              <label className="block text-sm font-medium text-gray-700 mb-1">
                Profile Picture (optional)
              </label>
              <input
                type="file"
                accept="image/*"
                onChange={handleImageUpload}
                className="w-full px-3 py-2 border border-gray-300 rounded-md focus:outline-none focus:ring-2 focus:ring-blue-500"
                disabled={isAuthenticating}
              />
              {formData.profilePicture && (
                <img 
                  src={formData.profilePicture} 
                  alt="Preview"
                  className="mt-2 w-16 h-16 rounded-full object-cover"
                />
              )}
            </div>

            <button
              type="submit"
              disabled={isAuthenticating}
              className="w-full bg-green-500 text-white py-3 px-4 rounded-md hover:bg-green-600 transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
            >
              {isAuthenticating ? 'Creating Account...' : 
                users.length === 0 ? '🔐 Create Account & Become Owner' : '🔐 Join Family'}
            </button>
          </form>

          {isPasskeySupported && (
            <div className="text-center">
              <p className="text-xs text-gray-500 mb-2">
                {users.length === 0 
                  ? 'A secure passkey will be created using your device\'s biometrics'
                  : 'You\'ll be prompted to use your fingerprint, face, or device PIN'
                }
              </p>
            </div>
          )}

          {!isPasskeySupported && (
            <div className="text-center">
              <p className="text-xs text-yellow-600 bg-yellow-50 p-2 rounded">
                ⚠️ Passkeys not supported. A fallback ID will be used.
              </p>
            </div>
          )}

          {window.location.hostname === 'localhost' && (
            <div className="text-center">
              <p className="text-xs text-blue-600 bg-blue-50 p-2 rounded">
                💡 For better passkey support, try accessing via 127.0.0.1 instead of localhost
              </p>
            </div>
          )}

          {/* Existing user login */}
          {users.length > 0 && (
            <div className="text-center border-t pt-4">
              <p className="text-sm text-gray-600 mb-2">Already have an account?</p>
              <button
                onClick={handlePasskeyLogin}
                disabled={isAuthenticating}
                className="text-blue-600 hover:text-blue-800 text-sm font-medium disabled:opacity-50"
              >
                🔐 Sign in with your passkey
              </button>
              <div className="mt-2">
                <button
                  onClick={() => {
                    PasskeyManager.debugStoredData();
                    console.log('Users in store:', users);
                  }}
                  className="text-xs text-gray-400 hover:text-gray-600"
                >
                  Debug Info
                </button>
              </div>
            </div>
          )}
        </div>
      )}
    </div>
  );
};