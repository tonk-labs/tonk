import React, { useState, useEffect } from 'react';
import { componentRegistry } from './ComponentRegistry';
import { sanitizeComponentName } from './contextBuilder';

export const AvailableComponentsPanel: React.FC = () => {
  const [components, setComponents] = useState(() =>
    componentRegistry.getAllComponents()
  );

  useEffect(() => {
    const updateComponents = () => {
      setComponents(componentRegistry.getAllComponents());
    };

    // Subscribe to context updates
    const unsubscribe = componentRegistry.onContextUpdate(updateComponents);

    // Also update on interval to catch any missed updates
    const interval = setInterval(updateComponents, 2000);

    return () => {
      unsubscribe();
      clearInterval(interval);
    };
  }, []);

  const availableNames = components
    .filter(comp => comp.metadata.status === 'success')
    .map(comp => sanitizeComponentName(comp.metadata.name))
    .filter(name => name !== 'UnnamedComponent');

  return (
    <div className="w-full bg-white border border-gray-200 p-4 overflow-y-auto">
      <h3 className="text-sm font-semibold text-gray-800 mb-3">
        Available Components ({availableNames.length})
      </h3>

      {availableNames.length === 0 ? (
        <p className="text-xs text-gray-500 italic">
          No components available yet. Create some components to see them here!
        </p>
      ) : (
        <div className="space-y-2">
          {availableNames.map(name => (
            <div key={name} className="text-xs">
              <code className="bg-blue-50 text-blue-700 px-2 py-1 rounded font-mono block">
                &lt;{name} /&gt;
              </code>
            </div>
          ))}
        </div>
      )}

      <div className="mt-4 pt-3 border-t border-gray-100">
        <p className="text-xs text-gray-500">
          Use these components directly in your code without imports. They
          update automatically when components change.
        </p>
      </div>
    </div>
  );
};
