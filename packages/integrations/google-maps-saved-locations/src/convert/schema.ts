export interface Location {
  addedBy: string;
  category: string;
  createdAt: number;
  description: string;
  id: string;
  latitude: number;
  longitude: number;
  name: string;
  placeId: string;
}

export interface OutputFormat {
  locations: {
    [key: string]: Location;
  };
  userNames: {
    [key: string]: string;
  };
}

export interface SchemaMapping {
  // Path to where locations should be stored in the document
  locationsPath: string[];
  // Path to where userNames should be stored in the document
  userNamesPath: string[];
  // Optional transformation function for location objects
  transformLocation?: (location: Location) => any;
  // Optional transformation function for userName objects
  transformUserName?: (userId: string, userName: string) => any;
}

// Default schema mapping that matches the original structure
export const DEFAULT_SCHEMA_MAPPING: SchemaMapping = {
  locationsPath: ['locations'],
  userNamesPath: ['userNames'],
};
