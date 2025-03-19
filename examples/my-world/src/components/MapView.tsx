import React, { useEffect, useState } from "react";
import { useLocationStore, useUserStore } from "../stores";
import { Plus, X, User, MapPin, Menu, ChevronLeft, Info } from "lucide-react";
import {
  MapContainer,
  TileLayer,
  Marker,
  Popup,
  useMapEvents,
  useMap,
  ZoomControl,
} from "react-leaflet";
import L from "leaflet";
import "leaflet/dist/leaflet.css";
import PlaceSearch from "./PlaceSearch";
import UserComparison from "./UserComparison";

// Fix Leaflet icon issue in React
// This is needed because of how webpack bundles assets
delete (L.Icon.Default.prototype as any)._getIconUrl;
L.Icon.Default.mergeOptions({
  iconRetinaUrl:
    "https://unpkg.com/leaflet@1.9.4/dist/images/marker-icon-2x.png",
  iconUrl: "https://unpkg.com/leaflet@1.9.4/dist/images/marker-icon.png",
  shadowUrl: "https://unpkg.com/leaflet@1.9.4/dist/images/marker-shadow.png",
});

// Component to handle map clicks and location setting
interface LocationPickerProps {
  isAddingLocation: boolean;
  onLocationPick: (lat: number, lng: number) => void;
}

const LocationPicker: React.FC<LocationPickerProps> = ({
  isAddingLocation,
  onLocationPick,
}) => {
  useMapEvents({
    click: (e) => {
      if (isAddingLocation) {
        onLocationPick(e.latlng.lat, e.latlng.lng);
      }
    },
  });

  return null;
};

// New component to control map view
interface MapControllerProps {
  center: [number, number] | null;
  zoom: number | null;
}

const MapController: React.FC<MapControllerProps> = ({ center, zoom }) => {
  const map = useMap();

  useEffect(() => {
    if (center) {
      map.setView(center, zoom || map.getZoom());
    }
  }, [center, zoom, map]);

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

  // Default map center
  const defaultCenter: [number, number] = [51.505, -0.09]; // London

  // Handle place selection from search
  const handlePlaceSelect = (
    latitude: number,
    longitude: number,
    name: string,
  ) => {
    setNewLocation({
      ...newLocation,
      name: name,
      latitude: latitude,
      longitude: longitude,
    });

    // Center the map on the selected location
    setMapCenter([latitude, longitude]);
    setMapZoom(15);
  };

  // Handle manual location pick from map click
  const handleLocationPick = (lat: number, lng: number) => {
    setNewLocation((prev) => ({
      ...prev,
      latitude: lat,
      longitude: lng,
    }));
  };

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
  };

  const cancelAddLocation = () => {
    setIsAddingLocation(false);
    setNewLocation({
      name: "",
      description: "",
      latitude: 0,
      longitude: 0,
    });
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
          <MapContainer
            center={defaultCenter}
            zoom={13}
            style={{ height: "100%", width: "100%" }}
            zoomControl={false}
          >
            <ZoomControl position="topright" />
            <TileLayer
              attribution='&copy; <a href="https://www.openstreetmap.org/copyright">OpenStreetMap</a> contributors'
              url="https://{s}.tile.openstreetmap.org/{z}/{x}/{y}.png"
            />

            <MapController center={mapCenter} zoom={mapZoom} />

            {/* Location markers */}
            {Object.values(locations).map((location) => {
              // Check if this location is in the common locations list
              const isCommonLocation = commonLocationIds.includes(location.id);

              // Determine marker color based on who added it and if it's common
              let markerColor = "blue"; // Default for current user
              if (userProfile.id !== location.addedBy) {
                markerColor = "red"; // Default for other users
              }
              if (isCommonLocation) {
                markerColor = "gold"; // For common locations
              }

              return (
                <Marker
                  key={location.id}
                  position={[location.latitude, location.longitude]}
                  icon={
                    new L.Icon({
                      iconUrl: isCommonLocation
                        ? "https://raw.githubusercontent.com/pointhi/leaflet-color-markers/master/img/marker-icon-2x-gold.png"
                        : userProfile.id === location.addedBy
                          ? "https://raw.githubusercontent.com/pointhi/leaflet-color-markers/master/img/marker-icon-2x-blue.png"
                          : "https://raw.githubusercontent.com/pointhi/leaflet-color-markers/master/img/marker-icon-2x-red.png",
                      shadowUrl:
                        "https://unpkg.com/leaflet@1.9.4/dist/images/marker-shadow.png",
                      iconSize: [25, 41],
                      iconAnchor: [12, 41],
                      popupAnchor: [1, -34],
                      shadowSize: [41, 41],
                    })
                  }
                >
                  <Popup>
                    <div>
                      <h3 className="font-semibold">{location.name}</h3>
                      <p className="text-sm">{location.description}</p>
                      <p className="text-xs text-gray-600">
                        Added by:{" "}
                        {userProfile.id === location.addedBy
                          ? "You"
                          : userNames[location.addedBy] || "Anonymous"}
                      </p>
                      {isCommonLocation && (
                        <p className="text-xs text-amber-600 font-medium mt-1">
                          ⭐ Common place
                        </p>
                      )}
                      {userProfile.id === location.addedBy && (
                        <button
                          onClick={() => removeLocation(location.id)}
                          className="text-xs text-red-500 mt-1"
                        >
                          Remove
                        </button>
                      )}
                    </div>
                  </Popup>
                </Marker>
              );
            })}

            {/* New Location Marker */}
            {isAddingLocation && newLocation.latitude !== 0 && (
              <Marker
                position={[newLocation.latitude, newLocation.longitude]}
                icon={
                  new L.Icon({
                    iconUrl:
                      "https://raw.githubusercontent.com/pointhi/leaflet-color-markers/master/img/marker-icon-2x-green.png",
                    shadowUrl:
                      "https://unpkg.com/leaflet@1.9.4/dist/images/marker-shadow.png",
                    iconSize: [25, 41],
                    iconAnchor: [12, 41],
                    popupAnchor: [1, -34],
                    shadowSize: [41, 41],
                  })
                }
              />
            )}

            {/* Map Event Handler */}
            <LocationPicker
              isAddingLocation={isAddingLocation}
              onLocationPick={handleLocationPick}
            />
          </MapContainer>

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
