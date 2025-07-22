export interface Location {
  id: string;
  name: string;
  address: string;
  latitude: number;
  longitude: number;
  description?: string;
  category: 'restaurant' | 'hotel' | 'attraction' | 'activity' | 'transport' | 'other';
  placeId?: string;
  addedBy: string;
  addedAt: string; // ISO string
}

export interface CalendarEvent {
  id: string;
  title: string;
  description?: string;
  startDate: string; // ISO string
  endDate: string; // ISO string
  locationId?: string;
  createdBy: string;
  createdAt: string; // ISO string
}

export interface PlaceSuggestion {
  id: string;
  name: string;
  description: string;
  location: Location;
  suggestedBy: string;
  suggestedAt: string; // ISO string
  votes: string[]; // user IDs who voted for this suggestion
  status: 'pending' | 'approved' | 'rejected';
}

export interface SharedNote {
  id: string;
  title: string;
  content: string;
  createdBy: string;
  createdAt: string; // ISO string
  lastModifiedBy: string;
  lastModifiedAt: string; // ISO string
  tags: string[];
}

export interface TripMember {
  id: string;
  name: string;
  joinedAt: string; // ISO string
}

export interface Trip {
  id: string;
  name: string;
  description: string;
  startDate: string; // ISO string
  endDate: string; // ISO string
  members: TripMember[];
  locations: Location[];
  events: CalendarEvent[];
  suggestions: PlaceSuggestion[];
  notes: SharedNote[];
  createdBy: string;
  createdAt: string; // ISO string
}