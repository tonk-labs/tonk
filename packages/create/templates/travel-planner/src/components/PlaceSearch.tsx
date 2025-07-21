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
    <div ref={searchRef} style={{ position: "relative", width: "100%" }} className={className}>
      <div
        style={{
          display: "flex",
          alignItems: "center",
          background: "rgba(255, 255, 255, 0.95)",
          border: `1px solid ${isFocused ? "rgba(0, 0, 0, 0.2)" : "rgba(0, 0, 0, 0.1)"}`,
          borderRadius: "8px",
          transition: "all 0.3s cubic-bezier(0.4, 0, 0.2, 1)",
          backdropFilter: "blur(10px)",
          boxShadow: isFocused ? "0 4px 12px rgba(0, 0, 0, 0.05)" : "0 2px 4px rgba(0, 0, 0, 0.1)"
        }}
      >
        <div style={{ paddingLeft: "0.75rem", paddingRight: "0.5rem", opacity: 0.6 }}>
          <Search size={16} />
        </div>
        <input
          type="text"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          onFocus={() => setIsFocused(true)}
          placeholder={placeholder}
          style={{
            padding: "0.5rem 0.25rem",
            width: "100%",
            outline: "none",
            fontSize: "0.875rem",
            background: "transparent",
            border: "none"
          }}
          aria-label="Search for a place"
        />
        {isLoading ? (
          <div style={{ padding: "0 0.75rem", opacity: 0.5 }}>
            <Loader size={16} style={{ animation: "spin 1s linear infinite" }} />
          </div>
        ) : (
          query && (
            <button
              style={{
                padding: "0 0.75rem",
                opacity: 0.5,
                background: "none",
                border: "none",
                cursor: "pointer",
                transition: "opacity 0.3s ease"
              }}
              onClick={() => {
                setQuery("");
                setResults([]);
              }}
              onMouseEnter={(e) => {
                e.currentTarget.style.opacity = "0.8";
              }}
              onMouseLeave={(e) => {
                e.currentTarget.style.opacity = "0.5";
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
        <div
          style={{
            position: "absolute",
            width: "100%",
            marginTop: "0.25rem",
            background: "rgba(255, 255, 255, 0.95)",
            borderRadius: "8px",
            border: "1px solid rgba(0, 0, 0, 0.1)",
            maxHeight: "15rem",
            overflowY: "auto",
            boxShadow: "0 8px 25px rgba(0, 0, 0, 0.1)",
            zIndex: 50,
            backdropFilter: "blur(10px)"
          }}
        >
          {results.map((result, index) => (
            <div
              key={index}
              style={{
                padding: "0.75rem",
                cursor: "pointer",
                transition: "all 0.3s ease",
                borderBottom: index < results.length - 1 ? "1px solid rgba(0, 0, 0, 0.05)" : "none",
                display: "flex",
                alignItems: "flex-start"
              }}
              onClick={() => handleResultSelect(result)}
              onMouseEnter={(e) => {
                e.currentTarget.style.background = "rgba(0, 0, 0, 0.03)";
              }}
              onMouseLeave={(e) => {
                e.currentTarget.style.background = "transparent";
              }}
            >
              <div style={{ marginRight: "0.75rem", marginTop: "0.25rem", fontSize: "0.875rem" }}>
                {getCategoryIcon(result.category || "")}
              </div>
              <div style={{ flex: 1 }}>
                <div style={{ fontWeight: "500", fontSize: "0.875rem" }}>{result.name}</div>
                <div style={{ fontSize: "0.75rem", opacity: 0.6, marginTop: "0.125rem" }}>
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