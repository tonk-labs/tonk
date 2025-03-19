import React, { useState, useEffect, useRef } from "react";
import { Search, X, Loader } from "lucide-react";

interface SearchResult {
  place_id: number;
  display_name: string;
  lat: string;
  lon: string;
}

interface PlaceSearchProps {
  onPlaceSelect: (latitude: number, longitude: number, name: string) => void;
}

const PlaceSearch: React.FC<PlaceSearchProps> = ({ onPlaceSelect }) => {
  const [query, setQuery] = useState("");
  const [results, setResults] = useState<SearchResult[]>([]);
  const [isLoading, setIsLoading] = useState(false);
  const [isOpen, setIsOpen] = useState(false);
  const searchRef = useRef<HTMLDivElement>(null);

  // Handle clicks outside the search component
  useEffect(() => {
    const handleClickOutside = (event: MouseEvent) => {
      if (
        searchRef.current &&
        !searchRef.current.contains(event.target as Node)
      ) {
        setIsOpen(false);
      }
    };

    document.addEventListener("mousedown", handleClickOutside);
    return () => {
      document.removeEventListener("mousedown", handleClickOutside);
    };
  }, []);

  // Search for places using Nominatim
  const searchPlaces = async (searchQuery: string) => {
    if (!searchQuery.trim()) {
      setResults([]);
      return;
    }

    setIsLoading(true);
    try {
      const response = await fetch(
        `https://nominatim.openstreetmap.org/search?format=json&q=${encodeURIComponent(
          searchQuery,
        )}&limit=5`,
      );

      if (response.ok) {
        const data: SearchResult[] = await response.json();
        setResults(data);
        setIsOpen(true);
      } else {
        console.error("Failed to fetch search results");
        setResults([]);
      }
    } catch (error) {
      console.error("Error searching places:", error);
      setResults([]);
    } finally {
      setIsLoading(false);
    }
  };

  // Handle search input changes with debounce
  useEffect(() => {
    const debounceTimeout = setTimeout(() => {
      if (query) {
        searchPlaces(query);
      }
    }, 500);

    return () => clearTimeout(debounceTimeout);
  }, [query]);

  // Handle selecting a search result
  const handleResultSelect = (result: SearchResult) => {
    onPlaceSelect(
      parseFloat(result.lat),
      parseFloat(result.lon),
      result.display_name.split(",")[0], // Just take the first part of the name for simplicity
    );
    setQuery("");
    setResults([]);
    setIsOpen(false);
  };

  return (
    <div ref={searchRef} className="relative search-container">
      <div className="flex items-center bg-white rounded-md shadow-sm border border-gray-300">
        <div className="pl-3 pr-2 text-gray-400">
          <Search size={18} />
        </div>
        <input
          type="text"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder="Search for a place..."
          className="py-2 px-1 w-full outline-none rounded-md"
        />
        {isLoading ? (
          <div className="px-3 text-gray-400">
            <Loader size={18} className="animate-spin" />
          </div>
        ) : (
          query && (
            <button
              className="px-3 text-gray-400 hover:text-gray-500"
              onClick={() => setQuery("")}
            >
              <X size={18} />
            </button>
          )
        )}
      </div>

      {/* Search Results Dropdown */}
      {isOpen && results.length > 0 && (
        <div className="absolute w-full mt-1 bg-white rounded-md shadow-lg border border-gray-200 max-h-60 overflow-y-auto search-results">
          {results.map((result) => (
            <div
              key={result.place_id}
              className="p-3 hover:bg-gray-100 cursor-pointer border-b border-gray-100 last:border-b-0"
              onClick={() => handleResultSelect(result)}
            >
              <div className="font-medium">
                {result.display_name.split(",")[0]}
              </div>
              <div className="text-xs text-gray-500">{result.display_name}</div>
            </div>
          ))}
        </div>
      )}
    </div>
  );
};

export default PlaceSearch;
