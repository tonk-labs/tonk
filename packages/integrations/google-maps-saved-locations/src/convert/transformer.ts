import { Client, PlaceInputType } from "@googlemaps/google-maps-services-js";
import { nanoid } from "nanoid";
import { OutputFormat } from "./schema";

export async function transformCsvToLocations(
  records: any[],
  apiKey: string,
  defaultUserId = "7fn52mcm1f5",
  defaultUserName = "Jack",
): Promise<OutputFormat> {
  // Initialize Google Maps client
  const client = new Client({});

  const output: OutputFormat = {
    locations: {},
    userNames: {
      [defaultUserId]: defaultUserName,
      n605c98uyhd: "Default User",
    },
  };

  // Process each location
  for (const record of records) {
    try {
      const searchResponse = await client.findPlaceFromText({
        params: {
          input: record.Title,
          inputtype: PlaceInputType.textQuery,
          fields: ["place_id", "name", "geometry"],
          key: apiKey,
        },
      });

      if (
        searchResponse.data.status !== "OK" ||
        !searchResponse.data.candidates ||
        searchResponse.data.candidates.length === 0
      ) {
        console.error(
          `Could not find place for: ${record.Title}`,
          searchResponse.data,
        );
        continue;
      }

      // Use the first result
      const candidate = searchResponse.data.candidates[0];
      console.log(`Found place for ${record.Title}: ${candidate.name}`);

      // Get detailed information using the place_id
      const response = await client.placeDetails({
        params: {
          place_id: candidate.place_id!,
          key: apiKey,
        },
      });

      if (response.data.status !== "OK") {
        console.error(`API error for ${record.Title}:`, response.data);
        continue;
      }

      const place = response.data.result;
      const locationId = nanoid(10);

      output.locations[locationId] = {
        addedBy: defaultUserId,
        category: "favorite", // Default category
        createdAt: Date.now(),
        description: record.Note || "",
        id: locationId,
        latitude: place.geometry?.location.lat || 0,
        longitude: place.geometry?.location.lng || 0,
        name: record.Title || place.name || "",
        placeId: place.place_id || "",
      };
    } catch (error) {
      console.error(`Error processing ${record.Title}:`, error);
    }
  }

  return output;
}
