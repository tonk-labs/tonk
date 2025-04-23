import React, { useState, useEffect, useRef } from "react";
import { Search, X, Loader } from "lucide-react";

// Define interface for Apple Maps search results
interface AppleSearchResult {
  displayLines: string[];
  coordinate: {
    latitude: number;
    longitude: number;
  };
  name: string;
  formattedAddress?: string;
  place?: any;
  placeId?: string;
  category?: string;
}

interface PlaceSearchProps {
  onPlaceSelect: (
    latitude: number,
    longitude: number,
    name: string,
    placeId?: string,
    place?: any,
  ) => void;
}

const PlaceSearch: React.FC<PlaceSearchProps> = ({ onPlaceSelect }) => {
  const [query, setQuery] = useState("");
  const [results, setResults] = useState<AppleSearchResult[]>([]);
  const [isLoading, setIsLoading] = useState(false);
  const [isFocused, setIsFocused] = useState(false);
  const searchRef = useRef<HTMLDivElement>(null);
  const searchInstance = useRef<any>(null);

  // Initialize MapKit search when component mounts
  useEffect(() => {
    if (window.mapkit && !searchInstance.current) {
      searchInstance.current = new window.mapkit.Search();
    }
  }, []);

  // Handle clicks outside the search component
  useEffect(() => {
    const handleClickOutside = (event: MouseEvent) => {
      if (
        searchRef.current &&
        !searchRef.current.contains(event.target as Node)
      ) {
        setIsFocused(false);
      }
    };

    document.addEventListener("mousedown", handleClickOutside);
    return () => {
      document.removeEventListener("mousedown", handleClickOutside);
    };
  }, []);

  // Search for places using Apple MapKit
  const searchPlaces = (searchQuery: string) => {
    if (!searchQuery.trim() || !window.mapkit) {
      setResults([]);
      return;
    }

    setIsLoading(true);

    if (!searchInstance.current) {
      searchInstance.current = new window.mapkit.Search({
        includePointsOfInterest: true,
        includeAddresses: true,
        includePhysicalFeatures: true,
        autocomplete: true,
      });
    }

    // Perform the search
    searchInstance.current.search(searchQuery, (error: any, data: any) => {
      setIsLoading(false);

      if (error) {
        console.error("Apple Maps search error:", error);
        setResults([]);
        return;
      }

      if (data && data.places) {
        // Transform theresults to our format
        const mappedResults: AppleSearchResult[] = data.places.map(
          (place: any) => ({
            displayLines: place.displayLines || [place.name],
            coordinate: place.coordinate,
            name: place.name,
            formattedAddress: place.formattedAddress,
            placeId: place.id,
            category: place.pointOfInterestCategory || "",
          }),
        );

        setResults(mappedResults);
      } else {
        setResults([]);
      }
    });
  };

  // Handle search input changes with debounce
  useEffect(() => {
    const debounceTimeout = setTimeout(() => {
      if (query) {
        searchPlaces(query);
      } else {
        setResults([]);
      }
    }, 300); // Reduced debounce time for more responsive feel

    return () => clearTimeout(debounceTimeout);
  }, [query]);

  // Handle selecting a search result
  const handleResultSelect = (result: AppleSearchResult) => {
    onPlaceSelect(
      result.coordinate.latitude,
      result.coordinate.longitude,
      result.name,
      result.placeId,
      result.place,
    );
    setQuery("");
    setResults([]);
  };

  // Get category icon (simplified version)
  const getCategoryIcon = (category: string) => {
    // This would be expanded with more category mappings in a real implementation
    if (category.includes("restaurant") || category.includes("food")) {
      return "🍽️";
    } else if (category.includes("hotel") || category.includes("lodging")) {
      return "🏨";
    } else if (
      category.includes("airport") ||
      category.includes("transportation")
    ) {
      return "✈️";
    } else if (category.includes("shopping") || category.includes("store")) {
      return "🛍️";
    } else if (category.includes("park") || category.includes("outdoor")) {
      return "🌳";
    }
    return "📍";
  };

  return (
    <div ref={searchRef} className="relative search-container w-full max-w-md">
      <div
        className={`flex items-center bg-white rounded-full shadow-lg transition-all duration-200 ${
          isFocused ? "ring-2 ring-blue-400" : ""
        }`}
        style={{
          boxShadow: "0 2px 12px rgba(0, 0, 0, 0.15)",
          backdropFilter: "blur(10px)",
          WebkitBackdropFilter: "blur(10px)",
        }}
      >
        <div className="pl-4 pr-2 text-gray-500">
          <Search size={16} />
        </div>
        <input
          type="text"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          onFocus={() => setIsFocused(true)}
          placeholder="Search Maps"
          className="py-3 px-1 w-full outline-none rounded-full text-sm font-medium"
          aria-label="Search for a place"
        />
        {isLoading ? (
          <div className="px-4 text-gray-400">
            <Loader size={16} className="animate-spin" />
          </div>
        ) : (
          query && (
            <button
              className="px-4 text-gray-400 hover:text-gray-600 transition-colors"
              onClick={() => setQuery("")}
              aria-label="Clear search"
            >
              <X size={16} />
            </button>
          )
        )}
      </div>

      {/* Search Results Dropdown - Apple-style */}
      {results.length > 0 && (
        <div
          className="absolute w-full mt-2 bg-white rounded-xl max-h-80 overflow-y-auto search-results z-10"
          style={{
            boxShadow: "0 4px 20px rgba(0, 0, 0, 0.15)",
            backdropFilter: "blur(10px)",
            WebkitBackdropFilter: "blur(10px)",
            border: "none",
          }}
        >
          {results.map((result, index) => (
            <div
              key={index}
              className="px-4 py-3 hover:bg-gray-50 cursor-pointer transition-colors duration-150 border-b border-gray-100 last:border-b-0 flex items-start"
              onClick={() => handleResultSelect(result)}
            >
              <div className="mr-3 mt-1 text-lg">
                {getCategoryIcon(result.category || "")}
              </div>
              <div className="flex-1">
                <div className="font-medium text-sm">{result.name}</div>
                <div className="text-xs text-gray-500 mt-0.5 line-clamp-2">
                  {result.formattedAddress || result.displayLines.join(", ")}
                </div>
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  );
};

export default PlaceSearch;
