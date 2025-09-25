import React from 'react';
import { Search } from 'lucide-react';

interface SearchInputProps {
  value: string;
  onChange: (value: string) => void;
  placeholder?: string;
  focusColor?: string; // For ring color on focus (e.g., 'blue', 'purple')
}

export const SearchInput: React.FC<SearchInputProps> = ({
  value,
  onChange,
  placeholder = 'Search...',
  focusColor = 'blue',
}) => {
  // Map color to complete class names (Tailwind requires complete class names)
  const focusClasses = {
    blue: 'focus:ring-2 focus:ring-blue-500',
    purple: 'focus:ring-2 focus:ring-purple-500',
    green: 'focus:ring-2 focus:ring-green-500',
  };

  const focusClass = focusClasses[focusColor as keyof typeof focusClasses] || focusClasses.blue;

  return (
    <div className="relative">
      <Search className="absolute left-3 top-1/2 transform -translate-y-1/2 text-gray-400 w-4 h-4" />
      <input
        type="text"
        placeholder={placeholder}
        value={value}
        onChange={e => onChange(e.target.value)}
        className={`w-full pl-10 pr-4 py-2 border border-gray-200 rounded-lg focus:outline-none ${focusClass} focus:border-transparent text-sm`}
      />
    </div>
  );
};