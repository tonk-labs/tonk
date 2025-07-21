import React, { useState } from "react";
import { useTripStore } from "../stores/tripStore";
import { CalendarEvent, PlaceSuggestion } from "../types/travel";
import { Plus, Edit, Trash2 } from "lucide-react";
import PlaceSearch from "./PlaceSearch";

interface CalendarProps {
  currentUser: string;
}

export function Calendar({ currentUser }: CalendarProps) {
  const { currentTrip, addEvent, removeEvent, updateEvent, addLocation } =
    useTripStore();
  const [selectedDate, setSelectedDate] = useState<Date>(new Date());
  const [showEventForm, setShowEventForm] = useState(false);
  const [editingEvent, setEditingEvent] = useState<CalendarEvent | null>(null);
  const [dragOverDate, setDragOverDate] = useState<Date | null>(null);
  const [eventForm, setEventForm] = useState({
    title: "",
    description: "",
    startDate: "",
    endDate: "",
    locationId: "",
    newLocationName: "",
    newLocationAddress: "",
    newLocationLatitude: 0,
    newLocationLongitude: 0,
    newLocationPlaceId: "",
    useNewLocation: false,
  });

  if (!currentTrip) return <div className="card">No trip selected</div>;

  const events = currentTrip.events || [];
  const locations = currentTrip.locations || [];

  const handleSubmitEvent = (e: React.FormEvent) => {
    e.preventDefault();

    let locationId = eventForm.locationId;

    // If using a new location, create it first
    if (
      eventForm.useNewLocation &&
      eventForm.newLocationName &&
      eventForm.newLocationLatitude !== 0
    ) {
      const newLocationId = Math.random().toString(36).substring(2, 11);
      addLocation({
        name: eventForm.newLocationName,
        address: eventForm.newLocationAddress,
        latitude: eventForm.newLocationLatitude,
        longitude: eventForm.newLocationLongitude,
        category: "other",
        placeId: eventForm.newLocationPlaceId,
        addedBy: currentUser,
      });
      locationId = newLocationId;
    }

    if (editingEvent) {
      updateEvent(editingEvent.id, {
        title: eventForm.title,
        description: eventForm.description,
        startDate: new Date(eventForm.startDate),
        endDate: new Date(eventForm.endDate),
        locationId: locationId || undefined,
      });
    } else {
      addEvent({
        title: eventForm.title,
        description: eventForm.description,
        startDate: new Date(eventForm.startDate),
        endDate: new Date(eventForm.endDate),
        locationId: locationId || undefined,
        createdBy: currentUser,
      });
    }

    setShowEventForm(false);
    setEditingEvent(null);
    setEventForm({
      title: "",
      description: "",
      startDate: "",
      endDate: "",
      locationId: "",
      newLocationName: "",
      newLocationAddress: "",
      newLocationLatitude: 0,
      newLocationLongitude: 0,
      newLocationPlaceId: "",
      useNewLocation: false,
    });
  };

  const handleEditEvent = (event: CalendarEvent) => {
    setEditingEvent(event);
    setEventForm({
      title: event.title,
      description: event.description || "",
      startDate: event.startDate.toISOString().slice(0, 16),
      endDate: event.endDate.toISOString().slice(0, 16),
      locationId: event.locationId || "",
      newLocationName: "",
      newLocationAddress: "",
      newLocationLatitude: 0,
      newLocationLongitude: 0,
      newLocationPlaceId: "",
      useNewLocation: false,
    });
    setShowEventForm(true);
  };

  const getWeekDays = (date: Date) => {
    const startOfWeek = new Date(date);
    const day = startOfWeek.getDay();
    const diff = startOfWeek.getDate() - day;
    startOfWeek.setDate(diff);

    const days = [];
    for (let i = 0; i < 7; i++) {
      const currentDay = new Date(startOfWeek);
      currentDay.setDate(startOfWeek.getDate() + i);
      days.push(currentDay);
    }

    return days;
  };

  const getEventsForDate = (date: Date) => {
    return events.filter((event) => {
      const eventDate = new Date(event.startDate);
      return eventDate.toDateString() === date.toDateString();
    });
  };

  const handleDrop = (e: React.DragEvent, date: Date) => {
    e.preventDefault();
    try {
      const data = JSON.parse(e.dataTransfer.getData("application/json"));
      const { item, type } = data;

      if (type === "suggestion") {
        const suggestion = item as PlaceSuggestion;
        // Create an event from the suggestion
        const startDate = new Date(date);
        startDate.setHours(9, 0, 0, 0); // Default to 9 AM
        const endDate = new Date(date);
        endDate.setHours(17, 0, 0, 0); // Default to 5 PM

        // First, ensure the location exists in the trip's locations
        const existingLocation = locations.find(
          (l) => l.id === suggestion.location.id,
        );
        if (!existingLocation) {
          addLocation({
            name: suggestion.location.name,
            address: suggestion.location.address,
            latitude: suggestion.location.latitude,
            longitude: suggestion.location.longitude,
            category: suggestion.location.category,
            placeId: suggestion.location.placeId,
            description: suggestion.location.description,
            addedBy: currentUser,
          });
        }

        addEvent({
          title: suggestion.name,
          description: suggestion.description,
          startDate,
          endDate,
          locationId: suggestion.location.id,
          createdBy: currentUser,
        });
      } else if (type === "event") {
        const event = item as CalendarEvent;
        // Move existing event to new date
        const timeDiff =
          new Date(event.endDate).getTime() -
          new Date(event.startDate).getTime();
        const newStartDate = new Date(date);
        newStartDate.setHours(
          new Date(event.startDate).getHours(),
          new Date(event.startDate).getMinutes(),
        );
        const newEndDate = new Date(newStartDate.getTime() + timeDiff);

        updateEvent(event.id, {
          startDate: newStartDate,
          endDate: newEndDate,
        });
      }
    } catch (error) {
      console.error("Error handling drop:", error);
    }
  };

  const handleDragLeave = () => {
    setDragOverDate(null);
  };

  const days = getWeekDays(selectedDate);
  const monthNames = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
  ];

  const formatWeekRange = (startDate: Date) => {
    const endDate = new Date(startDate);
    endDate.setDate(startDate.getDate() + 6);

    if (startDate.getMonth() === endDate.getMonth()) {
      return `${monthNames[startDate.getMonth()]} ${startDate.getDate()}-${endDate.getDate()}, ${startDate.getFullYear()}`;
    } else {
      return `${monthNames[startDate.getMonth()]} ${startDate.getDate()} - ${monthNames[endDate.getMonth()]} ${endDate.getDate()}, ${startDate.getFullYear()}`;
    }
  };

  return (
    <div className="card">
      <div
        className="list-item"
        style={{
          padding: "1rem 0",
          background: "transparent",
        }}
      >
        <h2>Trip Calendar</h2>
        <button
          onClick={() => setShowEventForm(true)}
          className="btn btn-primary"
          style={{ display: "flex", alignItems: "center", gap: "0.5rem" }}
        >
          <Plus size={20} />
          Add Event
        </button>
      </div>

      {/* Calendar Navigation */}
      <div className="calendar-navigation">
        <button
          onClick={() => {
            const newDate = new Date(selectedDate);
            newDate.setDate(selectedDate.getDate() - 7);
            setSelectedDate(newDate);
          }}
          className="btn btn-secondary"
        >
          ← Previous Week
        </button>
        <h3 className="calendar-week-title">
          {formatWeekRange(getWeekDays(selectedDate)[0])}
        </h3>
        <button
          onClick={() => {
            const newDate = new Date(selectedDate);
            newDate.setDate(selectedDate.getDate() + 7);
            setSelectedDate(newDate);
          }}
          className="btn btn-secondary"
        >
          Next Week →
        </button>
      </div>

      {/* Calendar Grid */}
      <div className="calendar-grid">
        {["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"].map((day) => (
          <div key={day} className="calendar-header-cell">
            {day}
          </div>
        ))}

        {days.map((day: Date, index: number) => (
          <div
            key={index}
            className={`calendar-day-cell ${
              dragOverDate && dragOverDate.toDateString() === day.toDateString()
                ? "drag-over"
                : ""
            }`}
            onDrop={(e) => {
              handleDrop(e, day);
              setDragOverDate(null);
            }}
            onDragOver={(e) => {
              e.preventDefault();
              e.dataTransfer.dropEffect = "move";
              setDragOverDate(day);
            }}
            onDragLeave={handleDragLeave}
          >
            <div className="calendar-day-header">
              {day.getDate()}
              <span className="calendar-day-header-weekday">
                {day.toLocaleDateString("en-US", { weekday: "short" })}
              </span>
            </div>
            {getEventsForDate(day).map((event) => (
              <div
                key={event.id}
                className="calendar-event"
                onClick={() => handleEditEvent(event)}
              >
                {event.title}
              </div>
            ))}
            {dragOverDate &&
              dragOverDate.toDateString() === day.toDateString() && (
                <div className="calendar-drop-zone">Drop here to schedule</div>
              )}
          </div>
        ))}
      </div>

      {/* Event Form Modal */}
      {showEventForm && (
        <div className="modal-overlay">
          <div className="modal-content">
            <h3 className="modal-title">
              {editingEvent ? "Edit Event" : "Add New Event"}
            </h3>

            <form onSubmit={handleSubmitEvent}>
              <div className="form-group">
                <label className="form-label">Event Title</label>
                <input
                  type="text"
                  value={eventForm.title}
                  onChange={(e) =>
                    setEventForm({ ...eventForm, title: e.target.value })
                  }
                  className="form-input"
                  required
                />
              </div>

              <div className="form-group">
                <label className="form-label">Description</label>
                <textarea
                  value={eventForm.description}
                  onChange={(e) =>
                    setEventForm({ ...eventForm, description: e.target.value })
                  }
                  className="form-textarea"
                  rows={3}
                />
              </div>

              <div className="form-group">
                <label className="form-label">Start Date & Time</label>
                <input
                  type="datetime-local"
                  value={eventForm.startDate}
                  onChange={(e) =>
                    setEventForm({ ...eventForm, startDate: e.target.value })
                  }
                  className="form-input"
                  required
                />
              </div>

              <div className="form-group">
                <label className="form-label">End Date & Time</label>
                <input
                  type="datetime-local"
                  value={eventForm.endDate}
                  onChange={(e) =>
                    setEventForm({ ...eventForm, endDate: e.target.value })
                  }
                  className="form-input"
                  required
                />
              </div>

              <div className="form-group">
                <label className="form-label">Location (Optional)</label>

                <div
                  style={{
                    display: "flex",
                    flexDirection: "column",
                    gap: "0.75rem",
                  }}
                >
                  {/* Existing locations */}
                  <div>
                    <select
                      value={
                        eventForm.useNewLocation ? "" : eventForm.locationId
                      }
                      onChange={(e) =>
                        setEventForm({
                          ...eventForm,
                          locationId: e.target.value,
                          useNewLocation: false,
                        })
                      }
                      className="form-select"
                      disabled={eventForm.useNewLocation}
                    >
                      <option value="">Select existing location</option>
                      {locations.map((location) => (
                        <option key={location.id} value={location.id}>
                          {location.name}
                        </option>
                      ))}
                    </select>
                  </div>

                  {/* Or divider */}
                  <div
                    style={{
                      textAlign: "center",
                      fontSize: "0.875rem",
                      opacity: 0.6,
                    }}
                  >
                    or
                  </div>

                  {/* New location search */}
                  <div>
                    <div
                      style={{
                        display: "flex",
                        alignItems: "center",
                        marginBottom: "0.5rem",
                      }}
                    >
                      <input
                        type="checkbox"
                        id="useNewLocation"
                        checked={eventForm.useNewLocation}
                        onChange={(e) =>
                          setEventForm({
                            ...eventForm,
                            useNewLocation: e.target.checked,
                            locationId: e.target.checked
                              ? ""
                              : eventForm.locationId,
                          })
                        }
                        style={{ marginRight: "0.5rem" }}
                      />
                      <label
                        htmlFor="useNewLocation"
                        className="form-label"
                        style={{ marginBottom: 0 }}
                      >
                        Add new location
                      </label>
                    </div>

                    {eventForm.useNewLocation && (
                      <div>
                        <PlaceSearch
                          onPlaceSelect={(
                            latitude,
                            longitude,
                            name,
                            address,
                            placeId,
                          ) => {
                            setEventForm({
                              ...eventForm,
                              newLocationName: name,
                              newLocationAddress: address,
                              newLocationLatitude: latitude,
                              newLocationLongitude: longitude,
                              newLocationPlaceId: placeId || "",
                            });
                          }}
                          placeholder="Search for a new location"
                        />
                        {eventForm.newLocationName && (
                          <div
                            style={{
                              marginTop: "0.5rem",
                              padding: "0.5rem",
                              background: "rgba(0, 0, 0, 0.02)",
                              borderRadius: "8px",
                              fontSize: "0.875rem",
                            }}
                          >
                            <div style={{ fontWeight: "500" }}>
                              {eventForm.newLocationName}
                            </div>
                            <div style={{ opacity: 0.7 }}>
                              {eventForm.newLocationAddress}
                            </div>
                          </div>
                        )}
                      </div>
                    )}
                  </div>
                </div>
              </div>

              <div className="modal-actions">
                <button
                  type="button"
                  onClick={() => {
                    setShowEventForm(false);
                    setEditingEvent(null);
                    setEventForm({
                      title: "",
                      description: "",
                      startDate: "",
                      endDate: "",
                      locationId: "",
                      newLocationName: "",
                      newLocationAddress: "",
                      newLocationLatitude: 0,
                      newLocationLongitude: 0,
                      newLocationPlaceId: "",
                      useNewLocation: false,
                    });
                  }}
                  className="btn btn-secondary"
                >
                  Cancel
                </button>
                <button type="submit" className="btn btn-primary">
                  {editingEvent ? "Update" : "Add"} Event
                </button>
              </div>
            </form>
          </div>
        </div>
      )}

      {/* Events List */}
      <div className="upcoming-events">
        <h3 className="upcoming-events-title">Upcoming Events</h3>
        <div className="upcoming-events-list">
          {events
            .filter((event) => new Date(event.startDate) >= new Date())
            .sort(
              (a, b) =>
                new Date(a.startDate).getTime() -
                new Date(b.startDate).getTime(),
            ).length > 0 ? (
            events
              .filter((event) => new Date(event.startDate) >= new Date())
              .sort(
                (a, b) =>
                  new Date(a.startDate).getTime() -
                  new Date(b.startDate).getTime(),
              )
              .map((event) => {
                const location = locations.find(
                  (l) => l.id === event.locationId,
                );
                return (
                  <div key={event.id} className="upcoming-event-item">
                    <div className="upcoming-event-content">
                      <div className="upcoming-event-details">
                        <h4 className="upcoming-event-title">{event.title}</h4>
                        {event.description && (
                          <p className="upcoming-event-description">
                            {event.description}
                          </p>
                        )}
                        <p className="upcoming-event-date">
                          {new Date(event.startDate).toLocaleDateString(
                            "en-US",
                            {
                              weekday: "short",
                              year: "numeric",
                              month: "short",
                              day: "numeric",
                            },
                          )}{" "}
                          -{" "}
                          {new Date(event.endDate).toLocaleDateString("en-US", {
                            weekday: "short",
                            year: "numeric",
                            month: "short",
                            day: "numeric",
                          })}
                        </p>
                        {location && (
                          <p className="upcoming-event-location">
                            📍 {location.name}
                          </p>
                        )}
                      </div>
                      <div className="upcoming-event-actions">
                        <button
                          onClick={() => handleEditEvent(event)}
                          className="upcoming-event-action-btn edit"
                          title="Edit event"
                        >
                          <Edit size={16} />
                        </button>
                        <button
                          onClick={() => removeEvent(event.id)}
                          className="upcoming-event-action-btn delete"
                          title="Delete event"
                        >
                          <Trash2 size={16} />
                        </button>
                      </div>
                    </div>
                  </div>
                );
              })
          ) : (
            <div className="no-upcoming-events">
              No upcoming events scheduled
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

