import { readDoc } from "@tonk/keepsync";
import { useLocationStore } from "../stores/locationStore";
import { useUserStore } from "../stores/userStore";

interface ImportedLocation {
  Title: string;
  URL: string;
  Comment?: string;
  Note?: string;
  exported_at: string;
  source_file: string;
}

interface LocationsDocument {
  locations: ImportedLocation[];
  metadata: {
    count: number;
    files: string[];
    lastUpdated: string;
  };
}

/**
 * Extracts place ID from Google Maps URL
 */
const extractPlaceIdFromUrl = (url: string): string | null => {
  // Example URL: https://www.google.com/maps/place/Clerkenwell+Workshops/data=!4m2!3m1!1s0x48761b4fea395e21:0xb85d538ede9f14
  const match = url.match(/!1s([^:]+):/);
  return match ? match[1] : null;
};

/**
 * Searches for a place using the MapKit search API
 */
const searchPlace = async (
  query: string,
  placeId?: string,
): Promise<{
  latitude: number;
  longitude: number;
  name: string;
  placeId?: string;
  category?: string;
} | null> => {
  return new Promise((resolve) => {
    if (!window.mapkit) {
      console.error("MapKit is not initialized");
      resolve(null);
      return;
    }

    const search = new window.mapkit.Search({
      includePointsOfInterest: true,
      includeAddresses: true,
    });

    search.search(query, (error: any, data: any) => {
      if (error) {
        console.error("Search error:", error);
        resolve(null);
        return;
      }

      if (data && data.places && data.places.length > 0) {
        const place = data.places[0];
        resolve({
          latitude: place.coordinate.latitude,
          longitude: place.coordinate.longitude,
          name: place.name,
          placeId: placeId || place.id,
          category: place.pointOfInterestCategory || "",
        });
      } else {
        console.warn(`No results found for "${query}"`);
        resolve(null);
      }
    });
  });
};

/**
 * Imports locations from a keepsync document
 */
export const importLocationsFromKeepsync = async (
  docPath: string,
  username: string = "Jack",
): Promise<{
  success: boolean;
  imported: number;
  skipped: number;
  failed: number;
  message: string;
}> => {
  try {
    // Get the document from keepsync
    const doc = await readDoc<LocationsDocument>(docPath);

    if (!doc || !doc.locations) {
      return {
        success: false,
        imported: 0,
        skipped: 0,
        failed: 0,
        message: "Failed to load locations document or no locations found",
      };
    }

    // Get the location store
    const locationStore = useLocationStore.getState();
    const userStore = useUserStore.getState();

    // Find or create user profile for 'Jack'
    let jackProfile = userStore.profiles.find(
      (profile) => profile.name === username,
    );

    if (!jackProfile) {
      jackProfile = userStore.createProfile(username);
      userStore.setActiveProfile(jackProfile.id);
    } else {
      userStore.setActiveProfile(jackProfile.id);
    }

    // Process each location
    let imported = 0;
    let skipped = 0;
    let failed = 0;

    // Get existing locations for duplicate checking
    const existingLocations = Object.values(locationStore.locations);

    for (const location of doc.locations) {
      try {
        // Extract place ID from URL if available
        const placeId = extractPlaceIdFromUrl(location.URL);

        // Check if location already exists by name or place ID
        const isDuplicate = existingLocations.some(
          (existingLocation) => 
            (existingLocation.name.toLowerCase() === location.Title.toLowerCase()) || 
            (placeId && existingLocation.placeId === placeId)
        );

        if (isDuplicate) {
          console.log(`Skipping duplicate location: "${location.Title}"`);
          skipped++;
          continue;
        }

        // Search for the place using MapKit
        const placeDetails = await searchPlace(
          location.Title,
          placeId || undefined,
        );

        if (placeDetails) {
          // Add the location to the store
          locationStore.addLocation({
            name: location.Title,
            description: location.Comment || location.Note || "",
            latitude: placeDetails.latitude,
            longitude: placeDetails.longitude,
            placeId: placeDetails.placeId || "",
            category: placeDetails.category || "other",
          });

          imported++;
        } else {
          console.warn(`Could not find coordinates for "${location.Title}"`);
          failed++;
        }
      } catch (error) {
        console.error(`Error importing location "${location.Title}":`, error);
        failed++;
      }
    }

    return {
      success: true,
      imported,
      skipped,
      failed,
      message: `Successfully imported ${imported} locations. ${skipped} locations skipped as duplicates. ${failed} locations failed to import.`,
    };
  } catch (error) {
    console.error("Error importing locations:", error);
    return {
      success: false,
      imported: 0,
      skipped: 0,
      failed: 0,
      message: `Error importing locations: ${error}`,
    };
  }
};
