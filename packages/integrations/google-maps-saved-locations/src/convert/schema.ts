export interface Location {
  name: string;
  description: string;
  placeId: string;
  googlePlaceDetails?: any;
}

export interface OutputFormat {
  locations: {
    [key: string]: Location;
  };
}
