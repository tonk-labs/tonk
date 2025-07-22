# How to create views
- Do not define new components, only use ones that already exist in the `src/components/` directory
- Use `div`s and `tailwind` to appropriately display components
- Always use semantic HTML elements (e.g., `main`, `section`, `article`, `nav`) for better accessibility
- All props should be explicitly typed with TypeScript
- Provide default values for optional props to prevent runtime errors
- Use descriptive prop names that indicate both type and purpose (e.g., `isVisible` not `flag`)

## Tailwind Usage
- Use only core Tailwind utility classes, no custom values
- Follow mobile-first responsive design using sm:, md:, lg: breakpoints
- Use semantic color classes (e.g., text-primary, bg-secondary) over literal colors
- Maintain consistent spacing scale using Tailwind's default spacing units

## State Management
- Use proper React hooks for lifecycle management (useEffect, useMemo, useCallback)
- All state that needs to be synchronized across clients should use keepsync stores
- All state that is relevant to the view and doesn't need to synchronize may simply call on useState
- All external functionality not related to rendering should be in a module
- Document all logic with explicit comments

## Accessibility
- Include ARIA labels and roles where appropriate
- Maintain proper heading hierarchy (h1 -> h6)
- Ensure sufficient color contrast using Tailwind's built-in colors
- Add keyboard navigation support for interactive elements

## Code Style
- Use explicit return statements for complex render logic
- Add JSDoc comments for component props and important functions
- Include example usage in comments for non-obvious implementations

## Examples

### Basic View Structure

Start with proper imports and component setup:

```typescript
/**
 * ProfileSettings view component
 * 
 * A view for managing user profile settings including personal information,
 * account preferences, and notification settings.
 * 
 * @example
 * <ProfileSettings userId="user-123" isEditable={true} />
 */

import React, { useState, useEffect, useMemo } from 'react';
import { useProfileStore } from '../stores/profileStore';
import { Avatar } from '../components/Avatar';
import { TextField } from '../components/TextField';
import { Button } from '../components/Button';

interface ProfileSettingsProps {
  userId: string;
  isEditable?: boolean;
}

export const ProfileSettings: React.FC<ProfileSettingsProps> = ({ 
  userId, 
  isEditable = true 
}) => {
  // Component implementation here...
};
```

### State Management Pattern

Use keepsync stores for synchronized data and local state for UI-specific data:

```typescript
export const ProfileSettings: React.FC<ProfileSettingsProps> = ({ 
  userId, 
  isEditable = true 
}) => {
  // Use store for synchronized state across clients
  // This data is automatically synced with the database by the store
  const { profile, updateProfile } = useProfileStore();
  
  // Use local state for form data (unsaved changes)
  const [formData, setFormData] = useState({
    name: '',
    email: '',
    bio: '',
    avatarUrl: ''
  });

  // UI-specific state that doesn't need to be synced
  const [isSaving, setIsSaving] = useState(false);
  const [errorMessage, setErrorMessage] = useState<string | null>(null);
  const [notifications, setNotifications] = useState({
    email: true,
    push: false,
    sms: false
  });

  // Load profile data on component mount or when userId changes
  useEffect(() => {
    if (profile) {
      setFormData({
        name: profile.name,
        email: profile.email,
        bio: profile.bio || '',
        avatarUrl: profile.avatarUrl || ''
      });
      
      setNotifications({
        email: profile.notificationPreferences?.email || false,
        push: profile.notificationPreferences?.push || false,
        sms: profile.notificationPreferences?.sms || false
      });
    }
  }, [profile, userId]);
};
```

### Event Handlers

Implement clean event handlers with proper error handling:

```typescript
const handleSave = async () => {
  if (!isEditable) return;
  
  setIsSaving(true);
  setErrorMessage(null);
  
  try {
    // Validate form data
    if (!formData.name.trim()) {
      throw new Error('Name is required');
    }
    
    if (!formData.email.trim()) {
      throw new Error('Email is required');
    }
    
    // Update profile through the store (automatically synced)
    await updateProfile({
      ...formData,
      notificationPreferences: notifications
    });
    
    console.log('Profile updated successfully');
  } catch (error) {
    setErrorMessage(error instanceof Error ? error.message : 'Failed to save profile');
    console.error('Profile update failed:', error);
  } finally {
    setIsSaving(false);
  }
};

const handleInputChange = (field: keyof typeof formData) => (value: string) => {
  setFormData(prev => ({
    ...prev,
    [field]: value
  }));
};

const handleNotificationChange = (type: keyof typeof notifications) => (enabled: boolean) => {
  setNotifications(prev => ({
    ...prev,
    [type]: enabled
  }));
};
```

### Render Logic

Structure your JSX with proper semantic HTML and Tailwind classes:

```typescript
return (
  <main className="max-w-2xl mx-auto p-6 bg-white rounded-lg shadow-sm">
    <header className="mb-8">
      <h1 className="text-2xl font-semibold text-gray-900 mb-2">
        Profile Settings
      </h1>
      <p className="text-gray-600">
        Manage your account information and preferences
      </p>
    </header>

    {errorMessage && (
      <div className="mb-6 p-4 bg-red-50 border border-red-200 rounded-md">
        <p className="text-red-700 text-sm">{errorMessage}</p>
      </div>
    )}

    <form onSubmit={(e) => { e.preventDefault(); handleSave(); }}>
      <section className="mb-8">
        <h2 className="text-lg font-medium text-gray-900 mb-4">
          Personal Information
        </h2>
        
        <div className="space-y-4">
          <div className="flex items-center gap-4 mb-6">
            <Avatar
              src={formData.avatarUrl}
              alt={formData.name}
              size="large"
              className="flex-shrink-0"
            />
            <div>
              <h3 className="text-sm font-medium text-gray-900">Profile Photo</h3>
              <p className="text-xs text-gray-500">
                Upload a new photo or change your existing one
              </p>
            </div>
          </div>

          <TextField
            label="Full Name"
            value={formData.name}
            onChange={handleInputChange('name')}
            placeholder="Enter your full name"
            required
            disabled={!isEditable}
          />

          <TextField
            label="Email Address"
            type="email"
            value={formData.email}
            onChange={handleInputChange('email')}
            placeholder="Enter your email"
            required
            disabled={!isEditable}
          />

          <TextField
            label="Bio"
            value={formData.bio}
            onChange={handleInputChange('bio')}
            placeholder="Tell us about yourself"
            multiline
            rows={3}
            disabled={!isEditable}
          />
        </div>
      </section>

      {isEditable && (
        <div className="flex justify-end gap-3">
          <Button
            type="button"
            variant="secondary"
            onClick={() => {
              // Reset form to original values
              if (profile) {
                setFormData({
                  name: profile.name,
                  email: profile.email,
                  bio: profile.bio || '',
                  avatarUrl: profile.avatarUrl || ''
                });
              }
            }}
            disabled={isSaving}
          >
            Cancel
          </Button>
          
          <Button
            type="submit"
            variant="primary"
            loading={isSaving}
            disabled={isSaving}
          >
            Save Changes
          </Button>
        </div>
      )}
    </form>
  </main>
);
```

## Best Practices Summary

1. **Component Structure**: Start with proper TypeScript interfaces and JSDoc comments
2. **State Management**: Use keepsync stores for synchronized data, local state for UI data
3. **Event Handlers**: Keep them clean with proper error handling and validation
4. **Accessibility**: Use semantic HTML elements and proper ARIA attributes
5. **Styling**: Follow Tailwind best practices with consistent spacing and responsive design
6. **Error Handling**: Provide clear error messages and loading states for better UX
