import React, { useEffect, useState, useRef } from "react";
import { useLocationStore, useUserStore } from "../stores";
import { Plus, X, User, MapPin, Menu, ChevronLeft, Info } from "lucide-react";
import PlaceSearch from "./PlaceSearch";
import UserComparison from "./UserComparison";

// Declare MapKit JS types
declare global {
  interface Window {
    mapkit: any;
  }
}

const getMapKitToken = async (): Promise<string> => {
  return "eyJraWQiOiJWWU5DUlVNTThHIiwidHlwIjoiSldUIiwiYWxnIjoiRVMyNTYifQ.eyJpc3MiOiI4V1ZLUzJGMjRDIiwiaWF0IjoxNzQyODM4MDI2LCJleHAiOjE3NDM0OTA3OTl9.PLqIZrssCXQPFXZ3OUn22EflQaxQbNcqDvbn2OMQNtF8HuSPfTzpyDL4zgsIlefeUJNuyZsZhGz4Baete43cFQ";
};

// Component to initialize MapKit JS
interface MapKitInitializerProps {
  onMapReady: (map: any) => void;
}

const MapKitInitializer: React.FC<MapKitInitializerProps> = ({}) => {
  useEffect(() => {
    const loadMapKit = async () => {
      try {
        // Load MapKit JS script if not already loaded
        if (!window.mapkit) {
          const script = document.createElement("script");
          script.src = "https://cdn.apple-mapkit.com/mk/5.x.x/mapkit.js";
          script.async = true;
          document.head.appendChild(script);

          await new Promise<void>((resolve) => {
            script.onload = () => resolve();
          });
        }

        // Initialize MapKit with JWT token
        const token = await getMapKitToken();
        window.mapkit.init({
          authorizationCallback: (done: (token: string) => void) => {
            done(token);
          },
        });
      } catch (error) {
        console.error("Failed to initialize MapKit JS:", error);
      }
    };

    loadMapKit();
  }, []);

  return null;
};

const MapView: React.FC = () => {
  const { locations, addLocation, removeLocation } = useLocationStore();
  const { profile: userProfile, setUserProfile } = useUserStore();
  const [isAddingLocation, setIsAddingLocation] = useState(false);
  const [newLocation, setNewLocation] = useState({
    name: "",
    description: "",
    latitude: 0,
    longitude: 0,
  });
  const [mapCenter, setMapCenter] = useState<[number, number] | null>(null);
  const [mapZoom, setMapZoom] = useState<number | null>(null);
  const [sidebarOpen, setSidebarOpen] = useState(false);
  const [isEditingProfile, setIsEditingProfile] = useState(false);
  const [profileName, setProfileName] = useState(userProfile.name);
  const [commonLocationIds, setCommonLocationIds] = useState<string[]>([]);
  const userNames = useLocationStore((state) => state.userNames);
  const mapRef = useRef<HTMLDivElement>(null);
  const mapInstanceRef = useRef<any>(null);
  const markersRef = useRef<any[]>([]);
  const [mapIsReady, setMapIsReady] = useState(false);

  // Default map center
  const defaultCenter: [number, number] = [51.505, -0.09]; // London

  // Initialize MapKit JS map
  useEffect(() => {
    if (window.mapkit && mapRef.current && !mapInstanceRef.current) {
      // Create a new MapKit JS map instance
      const map = new window.mapkit.Map(mapRef.current, {
        showsZoomControl: true,
        showsCompass: window.mapkit.FeatureVisibility.Adaptive,
        showsScale: window.mapkit.FeatureVisibility.Adaptive,
      });

      // Set initial region
      map.region = new window.mapkit.CoordinateRegion(
        new window.mapkit.Coordinate(defaultCenter[0], defaultCenter[1]),
        new window.mapkit.CoordinateSpan(0.1, 0.1),
      );

      // Add click event listener for adding new locations
      map.addEventListener("click", (event: any) => {
        if (isAddingLocation) {
          const coordinate = event.coordinate;
          handleLocationPick(coordinate.latitude, coordinate.longitude);
        }
      });

      mapInstanceRef.current = map;
      setMapIsReady(true);
    }
  }, [mapRef.current, window.mapkit]);

  // Update map when center or zoom changes
  useEffect(() => {
    if (mapIsReady && mapInstanceRef.current && mapCenter) {
      const map = mapInstanceRef.current;
      const zoomLevel = mapZoom || 15;
      const span = 0.01 * Math.pow(2, 15 - zoomLevel);

      map.region = new window.mapkit.CoordinateRegion(
        new window.mapkit.Coordinate(mapCenter[0], mapCenter[1]),
        new window.mapkit.CoordinateSpan(span, span),
      );
    }
  }, [mapCenter, mapZoom, mapIsReady]);

  // Handle place selection from search
  const handlePlaceSelect = (
    latitude: number,
    longitude: number,
    name: string,
  ) => {
    // Update state with the selected location
    setNewLocation({
      ...newLocation,
      name: name,
      latitude: latitude,
      longitude: longitude,
    });

    // Center the map on the selected location
    setMapCenter([latitude, longitude]);
    setMapZoom(15);

    // Add a simple temporary marker annotation
    if (mapIsReady && mapInstanceRef.current) {
      // Remove any existing temporary marker
      const tempMarker = markersRef.current.find((m) => m.isTemporary);
      if (tempMarker) {
        mapInstanceRef.current.removeAnnotation(tempMarker);
        markersRef.current = markersRef.current.filter((m) => !m.isTemporary);
      }

      // Create a simple marker annotation
      const coordinate = new window.mapkit.Coordinate(latitude, longitude);
      const marker = new window.mapkit.MarkerAnnotation(coordinate, {
        color: "#34C759", // Green color for new location
        title: name,
        glyphText: "+",
        // No selected property to avoid callout
      });

      // Mark it as temporary
      marker.isTemporary = true;

      // Add to map (but don't select it)
      try {
        mapInstanceRef.current.addAnnotation(marker);
        markersRef.current.push(marker);
      } catch (error) {
        console.error("Error adding annotation:", error);
      }
    }
  };

  // Handle manual location pick from map click
  const handleLocationPick = (lat: number, lng: number) => {
    setNewLocation((prev) => ({
      ...prev,
      latitude: lat,
      longitude: lng,
    }));

    // Add temporary marker for new location
    if (mapIsReady && mapInstanceRef.current) {
      // Remove any existing temporary marker
      const tempMarker = markersRef.current.find((m) => m.isTemporary);
      if (tempMarker) {
        mapInstanceRef.current.removeAnnotation(tempMarker);
        markersRef.current = markersRef.current.filter((m) => !m.isTemporary);
      }

      // Add new temporary marker
      const marker = new window.mapkit.MarkerAnnotation(
        new window.mapkit.Coordinate(lat, lng),
        {
          color: "#34C759", // Green color for new location
          title: "New Location",
          glyphText: "+",
        },
      );
      marker.isTemporary = true;

      mapInstanceRef.current.addAnnotation(marker);
      markersRef.current.push(marker);
    }
  };

  // Function to update map markers
  const updateMapMarkers = () => {
    if (!mapIsReady || !mapInstanceRef.current) return;

    // Remove all existing markers except temporary one
    const tempMarker = markersRef.current.find((m) => m.isTemporary);
    mapInstanceRef.current.removeAnnotations(
      markersRef.current.filter((m) => !m.isTemporary),
    );
    markersRef.current = tempMarker ? [tempMarker] : [];

    // Add markers for all locations
    const markers = Object.values(locations).map((location) => {
      // Determine marker color based on who added it and if it's common
      let markerColor = "#1D7AFF"; // Default blue for current user
      if (userProfile.id !== location.addedBy) {
        markerColor = "#FF3B30"; // Red for other users
      }
      if (commonLocationIds.includes(location.id)) {
        markerColor = "#FFCC00"; // Gold for common locations
      }

      const marker = new window.mapkit.MarkerAnnotation(
        new window.mapkit.Coordinate(location.latitude, location.longitude),
        {
          color: markerColor,
          title: location.name,
          subtitle: location.description || "",
          selected: false,
        },
      );

      // Add custom data to marker
      marker.locationId = location.id;

      // Add callout (popup) with more information
      marker.callout = {
        calloutElementForAnnotation: (annotation: any) => {
          const calloutElement = document.createElement("div");
          calloutElement.className = "mapkit-callout";
          calloutElement.style.padding = "10px";
          calloutElement.style.maxWidth = "200px";
          calloutElement.style.backgroundColor = "white";
          calloutElement.style.borderRadius = "8px";
          calloutElement.style.boxShadow = "0 2px 10px rgba(0, 0, 0, 0.1)";
          calloutElement.style.border = "1px solid rgba(0, 0, 0, 0.1)";

          const location = Object.values(locations).find(
            (loc) => loc.id === annotation.locationId,
          );
          if (!location) return calloutElement;

          calloutElement.innerHTML = `
                <h3 style="font-weight: 600; margin-bottom: 5px;">${location.name}</h3>
                ${location.description ? `<p style="font-size: 14px; margin-bottom: 5px;">${location.description}</p>` : ""}
                <p style="font-size: 12px; color: #666;">
                  Added by: ${userProfile.id === location.addedBy ? "You" : userNames[location.addedBy] || "Anonymous"}
                </p>
                ${
                  commonLocationIds.includes(location.id)
                    ? '<p style="font-size: 12px; color: #B8860B; font-weight: 500; margin-top: 5px;">⭐ Common place</p>'
                    : ""
                }
                ${
                  userProfile.id === location.addedBy
                    ? `<button id="remove-${location.id}" style="font-size: 12px; color: #FF3B30; margin-top: 5px; border: none; background: none; cursor: pointer; padding: 0;">Remove</button>`
                    : ""
                }
              `;

          // Add event listener for remove button
          setTimeout(() => {
            const removeButton = document.getElementById(
              `remove-${location.id}`,
            );
            if (removeButton) {
              removeButton.addEventListener("click", () => {
                removeLocation(location.id);
                mapInstanceRef.current.removeAnnotation(annotation);
                markersRef.current = markersRef.current.filter(
                  (m) => m !== annotation,
                );
              });
            }
          }, 0);

          return calloutElement;
        },
      };

      return marker;
    });

    // Add all markers to the map
    if (markers.length > 0) {
      mapInstanceRef.current.addAnnotations(markers);
      markersRef.current = [...markersRef.current, ...markers];
    }
  };

  // Update markers when locations or common locations change
  useEffect(() => {
    updateMapMarkers();
  }, [locations, commonLocationIds, mapIsReady]);

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    if (newLocation.name.trim() === "" || newLocation.latitude === 0) return;

    addLocation(newLocation);
    setIsAddingLocation(false);
    setNewLocation({
      name: "",
      description: "",
      latitude: 0,
      longitude: 0,
    });

    // Remove temporary marker
    if (mapIsReady && mapInstanceRef.current) {
      const tempMarker = markersRef.current.find((m) => m.isTemporary);
      if (tempMarker) {
        mapInstanceRef.current.removeAnnotation(tempMarker);
        markersRef.current = markersRef.current.filter((m) => !m.isTemporary);
      }
    }

    // Update markers to include the new location
    updateMapMarkers();
  };

  const cancelAddLocation = () => {
    setIsAddingLocation(false);
    setNewLocation({
      name: "",
      description: "",
      latitude: 0,
      longitude: 0,
    });

    // Remove temporary marker
    if (mapIsReady && mapInstanceRef.current) {
      const tempMarker = markersRef.current.find((m) => m.isTemporary);
      if (tempMarker) {
        mapInstanceRef.current.removeAnnotation(tempMarker);
        markersRef.current = markersRef.current.filter((m) => !m.isTemporary);
      }
    }
  };

  return (
    <div className="flex flex-col h-full relative">
      {/* Header - Responsive */}
      <div className="flex justify-between items-center p-4 bg-white shadow-sm">
        <div className="flex items-center gap-2">
          <button
            onClick={() => setSidebarOpen(!sidebarOpen)}
            className="md:hidden p-1 rounded-md hover:bg-gray-100"
            aria-label="Toggle sidebar"
          >
            <Menu className="h-5 w-5" />
          </button>
          <h2 className="text-xl font-semibold">My World</h2>
        </div>
        <div className="flex items-center gap-2">
          <User className="h-5 w-5" />
          <span className="inline">{userProfile.name}</span>
        </div>
      </div>

      {/* Mobile Sidebar Overlay */}
      {sidebarOpen && (
        <div
          className="md:hidden fixed inset-0 bg-black bg-opacity-50 z-[950]"
          onClick={() => setSidebarOpen(false)}
        />
      )}

      {/* Main Content Area */}
      <div className="relative flex flex-grow overflow-hidden">
        {/* Left Sidebar - Now includes profile for mobile */}
        <div
          className={`
            fixed md:relative top-0 h-full bg-white z-[960] overflow-y-auto
            w-[300px] shadow-lg transition-transform duration-300 ease-in-out
            ${sidebarOpen ? "translate-x-0" : "-translate-x-full md:translate-x-0"}
          `}
        >
          <div className="p-4 flex items-center justify-between md:hidden">
            <h2 className="font-semibold">Menu</h2>
            <button onClick={() => setSidebarOpen(false)}>
              <ChevronLeft />
            </button>
          </div>

          <div className="p-4">
            {/* Profile Section - Visible on mobile and desktop */}
            <div className="mb-6 p-4 bg-gray-50 rounded-lg">
              <h3 className="font-semibold mb-3 flex items-center gap-2">
                <User className="h-4 w-4" />
                Profile
              </h3>

              {isEditingProfile ? (
                <form
                  onSubmit={(e) => {
                    e.preventDefault();
                    if (profileName.trim() === "") return;
                    setUserProfile(profileName);
                    setIsEditingProfile(false);
                  }}
                  className="flex flex-col gap-3"
                >
                  <div>
                    <label
                      htmlFor="profile-name"
                      className="text-sm font-medium"
                    >
                      Name
                    </label>
                    <input
                      id="profile-name"
                      type="text"
                      value={profileName}
                      onChange={(e) => setProfileName(e.target.value)}
                      className="w-full px-3 py-2 border rounded-md mt-1"
                      placeholder="Your name"
                      required
                    />
                  </div>
                  <div className="flex gap-2 mt-2">
                    <button
                      type="submit"
                      className="flex-1 bg-green-500 text-white py-1 px-3 text-sm rounded hover:bg-green-600"
                    >
                      Save
                    </button>
                    <button
                      type="button"
                      onClick={() => {
                        setIsEditingProfile(false);
                        setProfileName(userProfile.name);
                      }}
                      className="flex-1 bg-gray-200 text-gray-800 py-1 px-3 text-sm rounded hover:bg-gray-300"
                    >
                      Cancel
                    </button>
                  </div>
                </form>
              ) : (
                <>
                  <div className="mb-2">
                    <div className="text-sm font-medium">Name</div>
                    <div>{userProfile.name}</div>
                  </div>
                  <div className="mb-2">
                    <div className="text-sm font-medium">User ID</div>
                    <div className="text-sm text-gray-500 truncate">
                      {userProfile.id}
                    </div>
                  </div>
                  <button
                    className="mt-2 text-sm text-blue-500 hover:text-blue-700"
                    onClick={() => {
                      setIsEditingProfile(true);
                      setProfileName(userProfile.name);
                    }}
                  >
                    Edit Profile
                  </button>
                </>
              )}
            </div>

            <UserComparison onShowCommonLocations={setCommonLocationIds} />

            {/* Saved Locations Section */}
            <div className="mb-6">
              <h3 className="font-medium mb-2 flex items-center gap-2">
                <MapPin className="h-4 w-4" />
                Saved Locations
              </h3>
              <div className="space-y-2 max-h-[40vh] overflow-y-auto">
                {Object.values(locations).length === 0 ? (
                  <p className="text-sm text-gray-500">
                    No locations saved yet
                  </p>
                ) : (
                  Object.values(locations).map((location) => (
                    <div
                      key={location.id}
                      className="p-2 bg-gray-50 rounded-md text-sm hover:bg-gray-100 cursor-pointer"
                      onClick={() => {
                        setMapCenter([location.latitude, location.longitude]);
                        setMapZoom(15);
                        setSidebarOpen(false);
                      }}
                    >
                      <div className="font-medium">{location.name}</div>
                      <div className="text-xs text-gray-500 truncate">
                        {location.description || "No description"}
                      </div>
                      <div className="text-xs text-gray-400 mt-1">
                        {userProfile.id === location.addedBy
                          ? "Added by you"
                          : `Added by ${userNames[location.addedBy] || "Anonymous"}`}
                      </div>
                    </div>
                  ))
                )}
              </div>
            </div>

            {/* About Section */}
            <div>
              <h3 className="font-medium mb-2 flex items-center gap-2">
                <Info className="h-4 w-4" />
                About
              </h3>
              <p className="text-sm text-gray-600">
                My World is a collaborative map where you can save and share
                locations with friends. All data is stored locally first and
                synced when online.
              </p>
            </div>
          </div>
        </div>

        {/* Map Container */}
        <div className="flex-grow h-full relative">
          <MapKitInitializer
            onMapReady={(map) => {
              mapInstanceRef.current = map;
              setMapIsReady(true);
            }}
          />

          {/* MapKit JS container */}
          <div
            ref={mapRef}
            style={{ height: "100%", width: "100%" }}
            className="map-container"
          />

          {/* FAB for adding locations */}
          {!isAddingLocation && (
            <button
              onClick={() => setIsAddingLocation(true)}
              className="absolute bottom-20 left-4 md:bottom-6 md:left-6 bg-blue-500 text-white p-3 md:p-4 rounded-full shadow-lg hover:bg-blue-600 transition-colors z-[900]"
              aria-label="Add location"
            >
              <Plus className="h-5 w-5 md:h-6 md:w-6" />
            </button>
          )}

          {/* Add Location Panel - Keep existing code */}
          {isAddingLocation && (
            <div className="absolute inset-x-0 bottom-0 bg-white rounded-t-xl shadow-lg z-[10000] transition-transform transform translate-y-0 max-h-[80vh] md:max-h-[60%] flex flex-col">
              {/* Keep existing panel code */}
              <div className="p-3 md:p-4 border-b flex items-center justify-between">
                <h3 className="font-semibold text-base md:text-lg">
                  Add New Location
                </h3>
                <button
                  onClick={cancelAddLocation}
                  className="p-1 rounded-full hover:bg-gray-100"
                >
                  <X className="h-5 w-5" />
                </button>
              </div>

              {/* Search Box */}
              <div className="p-3 md:p-4 border-b">
                <PlaceSearch onPlaceSelect={handlePlaceSelect} />
                <div className="mt-1 md:mt-2 text-xs md:text-sm text-gray-500">
                  Search for a place or tap directly on the map
                </div>
              </div>

              {/* Form */}
              <div className="p-3 md:p-4 overflow-y-auto">
                <form onSubmit={handleSubmit} className="flex flex-col gap-3">
                  <div>
                    <label
                      htmlFor="location-name"
                      className="block text-sm font-medium text-gray-700 mb-1"
                    >
                      Location Name
                    </label>
                    <input
                      id="location-name"
                      type="text"
                      placeholder="Enter a name for this location"
                      className="w-full px-3 py-2 border rounded-md"
                      value={newLocation.name}
                      onChange={(e) =>
                        setNewLocation((prev) => ({
                          ...prev,
                          name: e.target.value,
                        }))
                      }
                      required
                    />
                  </div>

                  <div>
                    <label
                      htmlFor="location-description"
                      className="block text-sm font-medium text-gray-700 mb-1"
                    >
                      Description (optional)
                    </label>
                    <textarea
                      id="location-description"
                      rows={3}
                      placeholder="What's special about this place?"
                      className="w-full px-3 py-2 border rounded-md"
                      value={newLocation.description}
                      onChange={(e) =>
                        setNewLocation((prev) => ({
                          ...prev,
                          description: e.target.value,
                        }))
                      }
                    />
                  </div>

                  {newLocation.latitude !== 0 && (
                    <div className="bg-blue-50 p-3 rounded-md flex items-center gap-2 text-sm">
                      <MapPin className="h-4 w-4 text-blue-500" />
                      <span>
                        Location selected at: {newLocation.latitude.toFixed(6)},{" "}
                        {newLocation.longitude.toFixed(6)}
                      </span>
                    </div>
                  )}

                  <div className="flex gap-3 mt-2">
                    <button
                      type="button"
                      onClick={cancelAddLocation}
                      className="flex-1 py-2 px-4 border border-gray-300 rounded-md text-gray-700 hover:bg-gray-50"
                    >
                      Cancel
                    </button>
                    <button
                      type="submit"
                      disabled={
                        newLocation.latitude === 0 || !newLocation.name.trim()
                      }
                      className={`flex-1 py-2 px-4 rounded-md ${
                        newLocation.latitude === 0 || !newLocation.name.trim()
                          ? "bg-gray-300 text-gray-500 cursor-not-allowed"
                          : "bg-green-500 text-white hover:bg-green-600"
                      }`}
                    >
                      Save Location
                    </button>
                  </div>
                </form>
              </div>
            </div>
          )}
        </div>
      </div>
    </div>
  );
};

export default MapView;
