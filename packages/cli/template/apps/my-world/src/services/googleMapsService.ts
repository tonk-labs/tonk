import { useLocationStore } from "../stores/locationStore";

// Define the interface for business hours
export interface BusinessHours {
  periods: {
    open: {
      day: number;
      time: string;
    };
    close: {
      day: number;
      time: string;
    };
  }[];
  weekdayText: string[];
  isOpen: boolean | null;
}

// Define the interface for place details response
interface PlaceDetailsResponse {
  result: {
    opening_hours?: {
      periods: {
        open: {
          day: number;
          time: string;
        };
        close: {
          day: number;
          time: string;
        };
      }[];
      weekday_text: string[];
      open_now?: boolean;
    };
  };
  status: string;
}

/**
 * Fetches business hours for a place using its Google Maps Place ID
 * @param placeId The Google Maps Place ID
 * @returns Promise with business hours or null if not available
 */
export const fetchBusinessHours = async (
  placeId: string,
): Promise<BusinessHours | null> => {
  if (!placeId) {
    console.warn("No place ID provided to fetch business hours");
    return null;
  }

  try {
    const apiKey = import.meta.env.GOOGLE_MAPS_API_KEY;

    if (!apiKey) {
      console.error("Google Maps API key not found in environment variables");
      return null;
    }

    const url = `https://maps.googleapis.com/maps/api/place/details/json?place_id=${placeId}&fields=opening_hours&key=${apiKey}`;

    // Use our proxy server to avoid CORS issues
    const proxyUrl = `http://localhost:3001/api/proxy?url=${encodeURIComponent(url)}`;

    const response = await fetch(proxyUrl);

    if (!response.ok) {
      throw new Error(`Failed to fetch place details: ${response.statusText}`);
    }

    const data: PlaceDetailsResponse = await response.json();

    if (data.status !== "OK") {
      console.error(`Google Maps API error: ${data.status}`, data);
      return null;
    }

    if (!data.result || !data.result.opening_hours) {
      console.warn("No opening hours available for this location");
      return null;
    }

    return {
      periods: data.result.opening_hours.periods,
      weekdayText: data.result.opening_hours.weekday_text,
      isOpen: data.result.opening_hours.open_now ?? null,
    };
  } catch (error) {
    console.error("Error fetching business hours:", error);
    return null;
  }
};

/**
 * Updates the isOpen status for all locations with place IDs
 */
export const updateAllLocationsOpenStatus = async (): Promise<void> => {
  const locationStore = useLocationStore.getState();
  const locations = locationStore.locations;

  // Process locations in batches to avoid rate limiting
  const batchSize = 5;
  const locationIds = Object.keys(locations).filter(
    (id) => locations[id].placeId,
  );

  for (let i = 0; i < locationIds.length; i += batchSize) {
    const batch = locationIds.slice(i, i + batchSize);

    // Process each location in the batch concurrently
    await Promise.all(
      batch.map(async (locationId) => {
        const location = locations[locationId];

        if (!location.placeId) return;

        try {
          const businessHours = await fetchBusinessHours(location.placeId);

          if (businessHours) {
            locationStore.updateLocation(locationId, {
              isOpen: businessHours.isOpen,
            });
          }
        } catch (error) {
          console.error(
            `Error updating open status for location ${locationId}:`,
            error,
          );
        }
      }),
    );

    // Add a small delay between batches to avoid rate limiting
    if (i + batchSize < locationIds.length) {
      await new Promise((resolve) => setTimeout(resolve, 1000));
    }
  }
};

/**
 * Fetches business hours for a specific location and updates the store
 * @param locationId The ID of the location to update
 * @returns Promise with the business hours or null
 */
export const fetchAndUpdateBusinessHours = async (
  locationId: string,
): Promise<BusinessHours | null> => {
  const locationStore = useLocationStore.getState();
  const location = locationStore.locations[locationId];

  if (!location || !location.placeId) {
    console.warn(`Location ${locationId} not found or has no place ID`);
    return null;
  }

  try {
    const businessHours = await fetchBusinessHours(location.placeId);

    if (businessHours) {
      locationStore.updateLocation(locationId, {
        isOpen: businessHours.isOpen,
      });
      return businessHours;
    }

    return null;
  } catch (error) {
    console.error(
      `Error fetching business hours for location ${locationId}:`,
      error,
    );
    return null;
  }
};
