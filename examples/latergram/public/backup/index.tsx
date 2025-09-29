export default function IndexPage() {
	// Define all events with their start times and durations
	const { events, startEditing } = useEventsStore();

	// State for EventEditor modal
	const [isEditorOpen, setIsEditorOpen] = React.useState(false);

	// State for EventDetailsModal
	const [isDetailsOpen, setIsDetailsOpen] = React.useState(false);
	const [selectedEvent, setSelectedEvent] = React.useState(null);

	// Find the earliest start time
	const earliestStart = Math.min(
		...events.map((event) => {
			console.log(event);
			return event.start;
		}),
	);
	const latestEnd = Math.max(
		...events.map((event) => event.start + event.duration),
	);

	console.log("TIMES", earliestStart, latestEnd);

	// Generate time slots from earliest to latest
	const startHour = Math.floor(earliestStart);
	const endHour = Math.ceil(latestEnd);
	const timeSlots = Array.from(
		{ length: endHour - startHour },
		(_, i) => startHour + i,
	);

	console.log("SLOT", timeSlots);

	// Function to format hour display
	const formatHour = (hour) => {
		if (hour === 0) return "12 AM";
		if (hour < 12) return `${hour} AM`;
		if (hour === 12) return "12 PM";
		return `${hour - 12} PM`;
	};

	// Function to get grid column position based on start time
	const getGridColumn = (startTime) => {
		return Math.round((startTime - startHour) * 4) + 1; // 4 columns per hour for 15-min precision
	};

	// Function to get grid column span based on duration
	const getGridColumnSpan = (duration) => {
		return Math.round(duration * 4); // 4 columns per hour for 15-min precision
	};

	// Group events by row
	const eventsByRow = events.reduce((acc, event) => {
		const row = event.row || 1;
		if (!acc[row]) acc[row] = [];
		acc[row].push(event);
		return acc;
	}, {});

	const totalColumns = (endHour - startHour) * 4; // 4 columns per hour for 15-min precision
	const maxRows = Math.max(...Object.keys(eventsByRow).map(Number));

	// Handle opening the event editor for new events
	const handleAddEvent = () => {
		startEditing(); // Start editing mode without a specific event (for new event)
		setIsEditorOpen(true);
	};

	// Handle opening the event editor for editing existing events
	const handleEditEvent = (event) => {
		startEditing(event);
		setIsEditorOpen(true);
	};

	// Handle closing the event editor
	const handleCloseEditor = () => {
		setIsEditorOpen(false);
	};

	// Handle showing event details
	const handleShowDetails = (event) => {
		setSelectedEvent(event);
		setIsDetailsOpen(true);
	};

	// Handle closing event details
	const handleCloseDetails = () => {
		setIsDetailsOpen(false);
		setSelectedEvent(null);
	};

	return (
		<div className="min-h-screen bg-gray-50 h-screen">
			{/* Header */}
			<header className="bg-white border-b border-gray-200 sticky top-0 z-10">
				<div className="px-4 py-4">
					<div className="flex items-center justify-between">
						<div className="flex items-center gap-3">
							<div className="w-8 h-8 bg-gradient-to-r from-purple-500 to-pink-500 rounded-lg flex items-center justify-center">
								<span className="text-white font-bold text-sm">E</span>
							</div>
							<h1 className="text-xl font-bold text-gray-900">
								Ensemble Schedule
							</h1>
						</div>
						<div className="flex items-center gap-4">
							<button
								onClick={handleAddEvent}
								className="px-4 py-2 bg-blue-600 text-white rounded-md hover:bg-blue-700 transition-colors text-sm font-medium flex items-center gap-2"
							>
								<svg
									className="w-4 h-4"
									fill="none"
									stroke="currentColor"
									viewBox="0 0 24 24"
								>
									<path
										strokeLinecap="round"
										strokeLinejoin="round"
										strokeWidth={2}
										d="M12 4v16m8-8H4"
									/>
								</svg>
								Add Event
							</button>
							<div className="text-sm text-gray-600">Friday, November 22</div>
						</div>
					</div>
				</div>
			</header>

			{/* Horizontal Scrollable Schedule */}
			<div className="overflow-x-auto">
				<div className="min-w-max p-4">
					{/* Time Grid Container */}
					<div
						className="grid shrink-0 min-w-full gap-0.5"
						style={{
							gridTemplateColumns: `repeat(${totalColumns}, minmax(30px, 30px))`,
							gridTemplateRows: `auto repeat(${maxRows}, 1fr)`,
						}}
					>
						{/* Time Headers */}
						{timeSlots.map((hour, index) => (
							<div
								key={hour}
								className="text-sm cursor-pointer flex items-center justify-center hover:bg-gray-100 font-semibold py-2 px-1 mx-0.5 lg:sticky lg:top-[4px] bg-white z-50 border border-solid border-neutral-300 transition-all duration-300 mb-0.5"
								style={{ gridColumn: `${index * 4 + 1} / span 4` }}
							>
								<div className="text-center">
									<div className="text-xs text-gray-600">
										{formatHour(hour)}
									</div>
								</div>
							</div>
						))}

						{/* Events */}
						{Object.entries(eventsByRow).map(([rowNumber, rowEvents]) =>
							rowEvents.map((event) => {
								const gridColumn = getGridColumn(event.start);
								const gridColumnSpan = getGridColumnSpan(event.duration);

								return (
									<EventBlock
										key={event.id}
										event={event}
										gridColumn={gridColumn}
										gridColumnSpan={gridColumnSpan}
										rowNumber={parseInt(rowNumber)}
										onEditEvent={handleEditEvent}
										onShowDetails={handleShowDetails}
									/>
								);
							}),
						)}
					</div>
				</div>
			</div>

			{/* Legend */}
			<div className="p-4 bg-white border-t border-gray-200 h-full">
				<div className="text-sm text-gray-600 mb-2">Event Types:</div>
				<div className="flex flex-wrap gap-4">
					<div className="flex items-center gap-2">
						<div className="w-4 h-4 bg-[rgba(255,133,166,0.3)] border-l-4 border-[rgba(255,133,166,1)]"></div>
						<span className="text-xs">Main Sessions</span>
					</div>
					<div className="flex items-center gap-2">
						<div className="w-4 h-4 bg-[rgba(116,172,223,0.3)] border-l-4 border-[rgba(116,172,223,1)]"></div>
						<span className="text-xs">Workshops</span>
					</div>
					<div className="flex items-center gap-2">
						<div className="w-4 h-4 bg-[rgba(34,197,94,0.3)] border-l-4 border-[rgba(34,197,94,1)]"></div>
						<span className="text-xs">Breaks & Social</span>
					</div>
					<div className="flex items-center gap-2">
						<div className="w-4 h-4 bg-[rgba(168,85,247,0.3)] border-l-4 border-[rgba(168,85,247,1)]"></div>
						<span className="text-xs">Creative Sessions</span>
					</div>
					<div className="flex items-center gap-2">
						<div className="w-4 h-4 bg-[rgba(139,69,19,0.3)] border-l-4 border-[rgba(139,69,19,1)]"></div>
						<span className="text-xs">Analytics</span>
					</div>
				</div>
			</div>

			{/* Event Editor Modal */}
			<EventEditor isOpen={isEditorOpen} onClose={handleCloseEditor} />

			{/* Event Details Modal */}
			{isDetailsOpen && selectedEvent && (
				<EventDetailsModal
					isOpen={isDetailsOpen}
					onClose={handleCloseDetails}
					event={selectedEvent}
				/>
			)}
		</div>
	);
}
