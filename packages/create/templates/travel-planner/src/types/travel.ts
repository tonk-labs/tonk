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
  addedAt: Date;
}

export interface CalendarEvent {
  id: string;
  title: string;
  description?: string;
  startDate: Date;
  endDate: Date;
  locationId?: string;
  createdBy: string;
  createdAt: Date;
}

export interface PlaceSuggestion {
  id: string;
  name: string;
  description: string;
  location: Location;
  suggestedBy: string;
  suggestedAt: Date;
  votes: string[]; // user IDs who voted for this suggestion
  status: 'pending' | 'approved' | 'rejected';
}

export interface SharedNote {
  id: string;
  title: string;
  content: string;
  createdBy: string;
  createdAt: Date;
  lastModifiedBy: string;
  lastModifiedAt: Date;
  tags: string[];
}

export interface TripMember {
  id: string;
  name: string;
  joinedAt: Date;
}

export interface Trip {
  id: string;
  name: string;
  description: string;
  startDate: Date;
  endDate: Date;
  members: TripMember[];
  locations: Location[];
  events: CalendarEvent[];
  suggestions: PlaceSuggestion[];
  notes: SharedNote[];
  createdBy: string;
  createdAt: Date;
}