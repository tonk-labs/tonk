import React, { useEffect, useState, useRef } from "react";
import { useTripStore } from "../stores/tripStore";
import { MapPin, X } from "lucide-react";
import PlaceSearch from "./PlaceSearch";
import styles from "./MapView.module.css";

// Declare MapKit JS types
declare global {
  interface Window {
    mapkit: any;
  }
}

const getMapKitToken = async (): Promise<string> => {
  const token =
    "eyJraWQiOiJNTDc0TlY3NjRDIiwidHlwIjoiSldUIiwiYWxnIjoiRVMyNTYifQ.eyJpc3MiOiI4V1ZLUzJGMjRDIiwiaWF0IjoxNzUzMTA5MzQ0LCJleHAiOjE3NTM3NzIzOTl9.ZmXUMN-JYMyRc6EB6XrDfxwgdfi1FqBx4LU7NuRfMXCIFLuVqrCOhTLOZqxIRbwuS3MQnrkvpU6gUwg2gmjUkw";

  if (!token) {
    console.error("MapKit token not found in environment variables");
    throw new Error(
      "MapKit token not configured. Please set MAPKIT_TOKEN environment variable.",
    );
  }

  return token;
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

interface MapViewProps {
  currentUser: string;
}

const MapView: React.FC<MapViewProps> = ({ currentUser }) => {
  const { currentTrip, addLocation, removeLocation } = useTripStore();
  const [isAddingLocation, setIsAddingLocation] = useState(false);
  const [newLocation, setNewLocation] = useState({
    name: "",
    description: "",
    address: "",
    latitude: 0,
    longitude: 0,
    placeId: "",
    category: "attraction" as const,
  });
  const [mapCenter, setMapCenter] = useState<[number, number] | null>(null);
  const [mapZoom, setMapZoom] = useState<number | null>(null);
  const mapRef = useRef<HTMLDivElement>(null);
  const mapInstanceRef = useRef<any>(null);
  const markersRef = useRef<any[]>([]);
  const [mapIsReady, setMapIsReady] = useState(false);

  if (!currentTrip) return <div>No trip selected</div>;

  const locations = currentTrip.locations || [];

  // Calculate map center based on trip locations
  const getMapCenter = (): [number, number] => {
    if (locations.length === 0) {
      // Default to London if no locations
      return [51.509865, -0.118092];
    }

    if (locations.length === 1) {
      // Center on the single location
      return [locations[0].latitude, locations[0].longitude];
    }

    // Calculate center point of all locations
    const avgLat =
      locations.reduce((sum, loc) => sum + loc.latitude, 0) / locations.length;
    const avgLng =
      locations.reduce((sum, loc) => sum + loc.longitude, 0) / locations.length;

    return [avgLat, avgLng];
  };

  const defaultCenter = getMapCenter();

  // Initialize MapKit JS map
  useEffect(() => {
    if (window.mapkit && mapRef.current && !mapInstanceRef.current) {
      // Set Apple Maps style options
      const colorScheme = window.matchMedia("(prefers-color-scheme: dark)")
        .matches
        ? window.mapkit.Map.ColorSchemes.Dark
        : window.mapkit.Map.ColorSchemes.Light;

      // Create a new MapKit JS map instance
      const map = new window.mapkit.Map(mapRef.current, {
        showsZoomControl: true,
        showsCompass: window.mapkit.FeatureVisibility.Adaptive,
        showsScale: window.mapkit.FeatureVisibility.Adaptive,
        showsMapTypeControl: false,
        isRotationEnabled: true,
        showsPointsOfInterest: true,
        showsUserLocation: true,
        colorScheme: colorScheme,
        padding: new window.mapkit.Padding({
          top: 50,
          right: 10,
          bottom: 50,
          left: 10,
        }),
      });

      // Apply Apple Maps styling
      map.mapType = window.mapkit.Map.MapTypes.Standard;

      // Set initial region with appropriate zoom
      const centerCoords = mapCenter || defaultCenter;

      // Calculate appropriate zoom level based on locations
      let span = 0.1; // Default span
      if (locations.length > 1) {
        // Calculate bounding box of all locations
        const lats = locations.map((loc) => loc.latitude);
        const lngs = locations.map((loc) => loc.longitude);
        const latSpan = Math.max(...lats) - Math.min(...lats);
        const lngSpan = Math.max(...lngs) - Math.min(...lngs);

        // Add padding and ensure minimum span
        span = Math.max(latSpan, lngSpan) * 1.5;
        span = Math.max(span, 0.01); // Minimum zoom
        span = Math.min(span, 2.0); // Maximum zoom out
      } else if (locations.length === 1) {
        span = 0.02; // Closer zoom for single location
      }

      map.region = new window.mapkit.CoordinateRegion(
        new window.mapkit.Coordinate(centerCoords[0], centerCoords[1]),
        new window.mapkit.CoordinateSpan(span, span),
      );

      // Add click event listener for adding new locations
      map.addEventListener("click", (event: any) => {
        const coordinate = event.coordinate;
        handleLocationPick(coordinate.latitude, coordinate.longitude);
        setIsAddingLocation(true);
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
    address: string,
    placeId?: string,
  ) => {
    setNewLocation({
      ...newLocation,
      name: name,
      address: address,
      latitude: latitude,
      longitude: longitude,
      placeId: placeId || "",
    });

    // Center the map on the selected location
    setMapCenter([latitude, longitude]);
    setMapZoom(15);

    // Add a temporary marker
    if (mapIsReady && mapInstanceRef.current) {
      // Remove any existing temporary marker
      const tempMarker = markersRef.current.find((m) => m.isTemporary);
      if (tempMarker) {
        mapInstanceRef.current.removeAnnotation(tempMarker);
        markersRef.current = markersRef.current.filter((m) => !m.isTemporary);
      }

      // Create a new temporary marker
      const coordinate = new window.mapkit.Coordinate(latitude, longitude);
      const marker = new window.mapkit.MarkerAnnotation(coordinate, {
        color: "#34C759", // Green color for new location
        title: name,
        glyphText: "+",
      });

      marker.isTemporary = true;
      mapInstanceRef.current.addAnnotation(marker);
      markersRef.current.push(marker);
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
    const markers = locations.map((location) => {
      // Determine marker color based on category
      let markerColor = "#007AFF"; // Default blue

      switch (location.category) {
        case "restaurant":
          markerColor = "#FF9500"; // Orange
          break;
        case "hotel":
          markerColor = "#5856D6"; // Purple
          break;
        case "attraction":
          markerColor = "#FF3B30"; // Red
          break;
        case "activity":
          markerColor = "#34C759"; // Green
          break;
        case "transport":
          markerColor = "#8E8E93"; // Gray
          break;
        default:
          markerColor = "#007AFF"; // Blue
      }

      // Create Apple-style marker annotation
      const marker = new window.mapkit.MarkerAnnotation(
        new window.mapkit.Coordinate(location.latitude, location.longitude),
        {
          color: markerColor,
          title: location.name,
          subtitle: location.description || location.address,
          selected: false,
          animates: true,
          displayPriority: 1000,
        },
      );

      // Add custom data to marker
      marker.locationId = location.id;

      // Add callout with more information
      marker.callout = {
        calloutElementForAnnotation: (annotation: any) => {
          const calloutElement = document.createElement("div");
          calloutElement.className = "mapkit-callout";
          calloutElement.style.padding = "16px";
          calloutElement.style.maxWidth = "280px";
          calloutElement.style.backgroundColor = "white";
          calloutElement.style.borderRadius = "14px";
          calloutElement.style.boxShadow = "0 4px 16px rgba(0, 0, 0, 0.12)";
          calloutElement.style.border = "none";
          calloutElement.style.fontFamily =
            "-apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, Helvetica, Arial, sans-serif";

          const loc = locations.find((l) => l.id === annotation.locationId);
          if (!loc) return calloutElement;

          calloutElement.innerHTML = `
            <h3 style="font-weight: 600; font-size: 17px; margin-bottom: 6px; color: #000;">${loc.name}</h3>
            ${loc.description ? `<p style="font-size: 15px; margin-bottom: 8px; color: #333;">${loc.description}</p>` : ""}
            <p style="font-size: 13px; color: #8E8E93; margin-bottom: 4px;">${loc.address}</p>
            <p style="font-size: 13px; color: #8E8E93; margin-bottom: 4px;">Category: ${loc.category}</p>
            <p style="font-size: 13px; color: #8E8E93; margin-bottom: 4px;">Added by: ${loc.addedBy}</p>
            <div style="display: flex; gap: 12px; margin-top: 12px;">
              <button id="remove-${loc.id}" style="font-size: 15px; color: #FF3B30; border: none; background: none; cursor: pointer; padding: 8px 12px; border-radius: 8px; font-weight: 500;">Remove</button>
            </div>
          `;

          // Add event listener for remove button
          setTimeout(() => {
            const removeButton = document.getElementById(`remove-${loc.id}`);
            if (removeButton) {
              removeButton.addEventListener("click", () => {
                removeLocation(loc.id);
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

  // Update markers when locations change
  useEffect(() => {
    updateMapMarkers();
  }, [locations, mapIsReady]);

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    if (newLocation.name.trim() === "" || newLocation.latitude === 0) return;

    addLocation({
      name: newLocation.name,
      description: newLocation.description || null,
      address: newLocation.address,
      latitude: newLocation.latitude,
      longitude: newLocation.longitude,
      category: newLocation.category,
      placeId: newLocation.placeId || null,
      addedBy: currentUser,
    });

    setIsAddingLocation(false);
    setNewLocation({
      name: "",
      description: "",
      address: "",
      latitude: 0,
      longitude: 0,
      placeId: "",
      category: "attraction",
    });

    // Remove temporary marker
    if (mapIsReady && mapInstanceRef.current) {
      const tempMarker = markersRef.current.find((m) => m.isTemporary);
      if (tempMarker) {
        mapInstanceRef.current.removeAnnotation(tempMarker);
        markersRef.current = markersRef.current.filter((m) => !m.isTemporary);
      }
    }

    updateMapMarkers();
  };

  const cancelAddLocation = () => {
    setIsAddingLocation(false);
    setNewLocation({
      name: "",
      description: "",
      address: "",
      latitude: 0,
      longitude: 0,
      placeId: "",
      category: "attraction",
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
    <div
      className="card"
      style={{ height: "80vh", minHeight: "700px", overflow: "hidden" }}
    >
      <div style={{ display: "flex", flexDirection: "column", height: "100%" }}>
        {/* Header */}
        <div
          style={{
            padding: "1rem",
            borderBottom: "1px solid rgba(0, 0, 0, 0.1)",
          }}
        >
          <div
            style={{
              display: "flex",
              justifyContent: "space-between",
              alignItems: "center",
            }}
          >
            <h2
              style={{ display: "flex", alignItems: "center", gap: "0.5rem" }}
            >
              <MapPin size={24} />
              Trip Map
            </h2>
            <div style={{ fontSize: "0.875rem", opacity: 0.7 }}>
              {locations.length} location{locations.length !== 1 ? "s" : ""}{" "}
              saved
            </div>
          </div>
        </div>

        {/* Map Container */}
        <div style={{ flex: 1, position: "relative" }}>
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
            className={styles.mapContainer}
          />

          {/* Search bar overlay */}
          <div
            style={{
              position: "absolute",
              top: "1rem",
              left: "1rem",
              zIndex: 10,
              width: "20rem",
            }}
          >
            <PlaceSearch
              onPlaceSelect={(latitude, longitude, name, address, placeId) => {
                handlePlaceSelect(latitude, longitude, name, address, placeId);
                setIsAddingLocation(true);
              }}
              placeholder="Search for places to add to your trip"
            />
          </div>

          {/* Add Location Panel */}
          {isAddingLocation && (
            <div className={styles.addLocationPanel}>
              <div className={styles.addLocationPanelContent}>
                <div className={styles.addLocationPanelHeader}>
                  <h3 className={styles.addLocationPanelTitle}>
                    Add Location to Trip
                  </h3>
                  <button
                    onClick={cancelAddLocation}
                    className={styles.addLocationPanelClose}
                  >
                    <X size={20} />
                  </button>
                </div>

                <form
                  onSubmit={handleSubmit}
                  className={styles.addLocationForm}
                >
                  <div className={styles.addLocationFormGroup}>
                    <label className={styles.addLocationFormLabel}>Name</label>
                    <input
                      type="text"
                      value={newLocation.name}
                      onChange={(e) =>
                        setNewLocation((prev) => ({
                          ...prev,
                          name: e.target.value,
                        }))
                      }
                      className={styles.addLocationFormInput}
                      placeholder="Location name"
                      required
                    />
                  </div>

                  <div className={styles.addLocationFormGroup}>
                    <label className={styles.addLocationFormLabel}>
                      Category
                    </label>
                    <select
                      value={newLocation.category}
                      onChange={(e) =>
                        setNewLocation((prev) => ({
                          ...prev,
                          category: e.target.value as any,
                        }))
                      }
                      className={styles.addLocationFormSelect}
                    >
                      <option value="attraction">Attraction</option>
                      <option value="restaurant">Restaurant</option>
                      <option value="hotel">Hotel</option>
                      <option value="activity">Activity</option>
                      <option value="transport">Transport</option>
                      <option value="other">Other</option>
                    </select>
                  </div>

                  <div
                    className={`${styles.addLocationFormGroup} ${styles.addLocationFormGroupFull}`}
                  >
                    <label className={styles.addLocationFormLabel}>
                      Description (optional)
                    </label>
                    <textarea
                      value={newLocation.description}
                      onChange={(e) =>
                        setNewLocation((prev) => ({
                          ...prev,
                          description: e.target.value,
                        }))
                      }
                      className={styles.addLocationFormTextarea}
                      rows={2}
                      placeholder="What's special about this place?"
                    />
                  </div>

                  <div className={styles.addLocationFormActions}>
                    <button
                      type="button"
                      onClick={cancelAddLocation}
                      className={styles.addLocationFormCancel}
                    >
                      Cancel
                    </button>
                    <button
                      type="submit"
                      disabled={
                        newLocation.latitude === 0 || !newLocation.name.trim()
                      }
                      className={styles.addLocationFormSubmit}
                    >
                      Add Location
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
