// create and sync are available in the compilation context

interface Event {
	id: number;
	title: string;
	start: number; // Start time in hours (e.g., 6.5 for 6:30 AM)
	duration: number; // Duration in hours (e.g., 0.5 for 30 minutes)
	type: "main" | "workshop" | "break" | "social" | "creative" | "analytics";
	organizer: string;
	row?: number; // Grid row for stacking overlapping events
}

interface EventsState {
	events: Event[];
	isEditing: boolean;
	editingEvent: Event | null;

	// Actions
	addEvent: (event: Omit<Event, "id">) => void;
	updateEvent: (id: number, updates: Partial<Event>) => void;
	deleteEvent: (id: number) => void;
	startEditing: (event?: Event) => void;
	stopEditing: () => void;
	getEventsByTimeRange: (startHour: number, endHour: number) => Event[];
	getEarliestEventTime: () => number | null;
	getLatestEventTime: () => number | null;
}

const useEventsStore = create<EventsState>()(
	sync(
		(set, get) => ({
			events: [
				{
					id: 1,
					title: "Welcome to Latergram",
					start: 6.5,
					duration: 0.5,
					type: "main",
					organizer: "LATERGRAM TEAM",
				},
				{
					id: 2,
					title: "Content Strategy Workshop",
					start: 8,
					duration: 2,
					type: "workshop",
					organizer: "CONTENT TEAM",
				},
				{
					id: 3,
					title: "Brand Voice Workshop",
					start: 8,
					duration: 1,
					type: "main",
					organizer: "EMMA GARCIA",
					row: 2,
				},
				{
					id: 4,
					title: "Coffee Break",
					start: 10,
					duration: 0.17,
					type: "break",
					organizer: "BREAK",
				},
				{
					id: 5,
					title: "One-on-One Sessions",
					start: 9,
					duration: 0.5,
					type: "social",
					organizer: "MENTORS",
					row: 3,
				},
				{
					id: 6,
					title: "Instagram Stories Mastery",
					start: 11,
					duration: 0.75,
					type: "workshop",
					organizer: "SARAH CHEN",
				},
				{
					id: 7,
					title: "Template Library Tour",
					start: 11,
					duration: 0.5,
					type: "creative",
					organizer: "DESIGN TEAM",
					row: 2,
				},
				{
					id: 8,
					title: "Lunch Break",
					start: 12,
					duration: 1,
					type: "break",
					organizer: "BREAK",
				},
				{
					id: 9,
					title: "Analytics Deep Dive",
					start: 14,
					duration: 1.5,
					type: "analytics",
					organizer: "DATA TEAM",
				},
				{
					id: 10,
					title: "Advanced Metrics",
					start: 14,
					duration: 0.75,
					type: "analytics",
					organizer: "ANALYTICS PRO",
					row: 2,
				},
				{
					id: 11,
					title: "Video Content Bootcamp",
					start: 16,
					duration: 1.25,
					type: "creative",
					organizer: "MIKE TORRES",
				},
				{
					id: 12,
					title: "Editing Masterclass",
					start: 16,
					duration: 1,
					type: "creative",
					organizer: "CREATIVE TEAM",
					row: 2,
				},
				{
					id: 13,
					title: "TikTok Trends",
					start: 15,
					duration: 0.5,
					type: "creative",
					organizer: "TREND TEAM",
					row: 3,
				},
				{
					id: 14,
					title: "Automation Setup",
					start: 18,
					duration: 0.75,
					type: "workshop",
					organizer: "TECH TEAM",
				},
				{
					id: 15,
					title: "Q&A Session",
					start: 19,
					duration: 0.5,
					type: "main",
					organizer: "ALL SPEAKERS",
				},
				{
					id: 16,
					title: "Networking Hour",
					start: 20,
					duration: 1,
					type: "social",
					organizer: "SOCIAL",
				},
			],
			isEditing: false,
			editingEvent: null,

			addEvent: (eventData) => {
				const newEvent: Event = {
					...eventData,
					id: Date.now(),
				};

				set((state) => ({
					events: [...state.events, newEvent],
				}));
			},

			updateEvent: (id, updates) => {
				set((state) => ({
					events: state.events.map((event) =>
						event.id === id ? { ...event, ...updates } : event,
					),
				}));
			},

			deleteEvent: (id) => {
				set((state) => ({
					events: state.events.filter((event) => event.id !== id),
				}));
			},

			startEditing: (event) => {
				set({
					isEditing: true,
					editingEvent: event || null,
				});
			},

			stopEditing: () => {
				set({
					isEditing: false,
					editingEvent: null,
				});
			},

			getEventsByTimeRange: (startHour, endHour) => {
				const { events } = get();
				return events.filter((event) => {
					const eventEndTime = event.start + event.duration;
					return event.start >= startHour && eventEndTime <= endHour;
				});
			},

			getEarliestEventTime: () => {
				const { events } = get();
				if (events.length === 0) return null;
				return Math.min(...events.map((event) => event.start));
			},

			getLatestEventTime: () => {
				const { events } = get();
				if (events.length === 0) return null;
				return Math.max(...events.map((event) => event.start + event.duration));
			},
		}),
		{ path: "/src/stores/events-store.json" },
	),
);

export default useEventsStore;
