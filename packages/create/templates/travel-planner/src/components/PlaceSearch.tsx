import React, { useState, useEffect, useRef } from "react";
import { Search, X, Loader } from "lucide-react";

// Declare MapKit JS types
declare global {
  interface Window {
    mapkit: any;
  }
}

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
    address: string,
    placeId?: string,
    place?: any,
  ) => void;
  placeholder?: string;
  className?: string;
}

const PlaceSearch: React.FC<PlaceSearchProps> = ({ 
  onPlaceSelect, 
  placeholder = "Search for a place",
  className = ""
}) => {
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
        // Transform the results to our format
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
    }, 300);

    return () => clearTimeout(debounceTimeout);
  }, [query]);

  // Handle selecting a search result
  const handleResultSelect = (result: AppleSearchResult) => {
    onPlaceSelect(
      result.coordinate.latitude,
      result.coordinate.longitude,
      result.name,
      result.formattedAddress || result.displayLines.join(", "),
      result.placeId,
      result.place,
    );
    setQuery("");
    setResults([]);
    setIsFocused(false);
  };

  // Get category icon
  const getCategoryIcon = (category: string) => {
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
    <div ref={searchRef} className={`place-search ${className}`}>
      <div className={`place-search-input-container ${isFocused ? 'focused' : ''}`}>
        <div className="place-search-icon">
          <Search size={16} />
        </div>
        <input
          type="text"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          onFocus={() => setIsFocused(true)}
          placeholder={placeholder}
          className="place-search-input"
          aria-label="Search for a place"
        />
        {isLoading ? (
          <div className="place-search-loading">
            <Loader size={16} className="place-search-spinner" />
          </div>
        ) : (
          query && (
            <button
              className="place-search-clear"
              onClick={() => {
                setQuery("");
                setResults([]);
              }}
              aria-label="Clear search"
            >
              <X size={16} />
            </button>
          )
        )}
      </div>

      {/* Search Results Dropdown */}
      {results.length > 0 && isFocused && (
        <div className="place-search-results">
          {results.map((result, index) => (
            <div
              key={index}
              className={`place-search-result-item ${index < results.length - 1 ? 'with-border' : ''}`}
              onClick={() => handleResultSelect(result)}
            >
              <div className="place-search-result-icon">
                {getCategoryIcon(result.category || "")}
              </div>
              <div className="place-search-result-content">
                <div className="place-search-result-name">{result.name}</div>
                <div className="place-search-result-address">
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