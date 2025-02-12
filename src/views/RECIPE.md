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
- Always destructure state values from hooks at the top of components
- Use proper React hooks for lifecycle management (useEffect, useMemo, useCallback)
- Do not over-engineer state management, do not create unnecessary complexity
- Document state dependencies in useEffect hooks with explicit comments

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
### Profile View
```tsx
import React from 'react';
import { Card, CardHeader, CardContent } from '@/components/ui/card';
import { Avatar, AvatarImage, AvatarFallback } from '@/components/ui/avatar';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { useToast } from '@/components/ui/use-toast';
import { Mail } from 'lucide-react';

/**
 * Displays a user profile card with avatar, basic info, and actions
 * Demonstrates proper semantic HTML, accessibility, and Tailwind usage
 */
const ProfileView = () => {
  const { toast } = useToast();

  const handleContact = () => {
    toast({
      title: "Contact initiated",
      description: "Message sent to user's inbox"
    });
  };

  return (
    <main className="min-h-screen bg-gray-50 p-4 md:p-6 lg:p-8">
      <section aria-label="User Profile" className="max-w-2xl mx-auto">
        <Card>
          <CardHeader>
            <div className="flex items-center space-x-4">
              <Avatar className="h-12 w-12">
                <AvatarImage
                  src="/api/placeholder/100/100"
                  alt="User profile picture"
                />
                <AvatarFallback>JD</AvatarFallback>
              </Avatar>
              <div>
                <h1 className="text-2xl font-semibold text-gray-900">
                  Jane Doe
                </h1>
                <p className="text-sm text-gray-500">
                  Senior Software Engineer
                </p>
              </div>
            </div>
          </CardHeader>

          <CardContent>
            <div className="space-y-4">
              {/* Skills section */}
              <div className="space-y-2">
                <h2 className="text-lg font-medium text-gray-900">
                  Skills
                </h2>
                <div className="flex flex-wrap gap-2">
                  <Badge>React</Badge>
                  <Badge>TypeScript</Badge>
                  <Badge>Node.js</Badge>
                  <Badge>AWS</Badge>
                </div>
              </div>

              {/* Bio section */}
              <article className="prose prose-sm">
                <h2 className="text-lg font-medium text-gray-900">
                  About
                </h2>
                <p className="text-gray-600">
                  Full-stack developer with 5 years of experience building scalable web applications.
                  Passionate about clean code and user experience.
                </p>
              </article>

              {/* Actions */}
              <div className="pt-4">
                <Button
                  onClick={handleContact}
                  className="w-full sm:w-auto"
                  aria-label="Contact Jane Doe"
                >
                  <Mail className="mr-2 h-4 w-4" />
                  Contact
                </Button>
              </div>
            </div>
          </CardContent>
        </Card>
      </section>
    </main>
  );
};

export default ProfileView;
```
