import React from "react";
import { useTripStore } from "../stores/tripStore";
import { CalendarEvent, PlaceSuggestion } from "../types/travel";
import { Calendar, MapPin, Clock, User } from "lucide-react";

interface PlansEventsListProps {
  currentUser: string;
}

export function PlansEventsList({}: PlansEventsListProps) {
  const { currentTrip } = useTripStore();

  if (!currentTrip) return <div>No trip selected</div>;

  const events = currentTrip.events || [];
  const approvedSuggestions = (currentTrip.suggestions || []).filter(
    (s) => s.status === "approved",
  );
  const locations = currentTrip.locations || [];

  const getLocationName = (locationId?: string) => {
    if (!locationId) return null;
    const location = locations.find((l) => l.id === locationId);
    return location?.name;
  };

  const handleDragStart = (
    e: React.DragEvent,
    item: CalendarEvent | PlaceSuggestion,
    type: "event" | "suggestion",
  ) => {
    e.dataTransfer.setData("application/json", JSON.stringify({ item, type }));
    e.dataTransfer.effectAllowed = "move";
  };

  return (
    <div className="card">
      <h2>Plans</h2>

      {/* Scheduled Events */}
      <div style={{ marginBottom: "2rem" }}>
        <h3 style={{ display: "flex", alignItems: "center", gap: "0.5rem", marginBottom: "1rem" }}>
          <Calendar size={20} />
          Scheduled Events ({events.length})
        </h3>
        <div>
          {events.length === 0 ? (
            <div className="empty-state">
              <div className="empty-state-icon">
                <Calendar size={32} />
              </div>
              <h3>No scheduled events yet</h3>
              <p>Drag approved suggestions to the calendar to schedule them!</p>
            </div>
          ) : (
            events
              .sort(
                (a, b) =>
                  new Date(a.startDate).getTime() -
                  new Date(b.startDate).getTime(),
              )
              .map((event) => (
                <div
                  key={event.id}
                  draggable
                  onDragStart={(e) => handleDragStart(e, event, "event")}
                  className="list-item"
                  style={{ cursor: "move" }}
                >
                  <div className="list-item-content">
                    <div className="list-item-title">{event.title}</div>
                    {event.description && (
                      <div className="list-item-subtitle">{event.description}</div>
                    )}
                    <div style={{ display: "flex", alignItems: "center", gap: "1rem", marginTop: "0.5rem", fontSize: "0.875rem", opacity: 0.7 }}>
                      <div style={{ display: "flex", alignItems: "center", gap: "0.25rem" }}>
                        <Clock size={14} />
                        <span>
                          {new Date(event.startDate).toLocaleDateString()} -{" "}
                          {new Date(event.endDate).toLocaleDateString()}
                        </span>
                      </div>
                      {getLocationName(event.locationId) && (
                        <div style={{ display: "flex", alignItems: "center", gap: "0.25rem" }}>
                          <MapPin size={14} />
                          <span>{getLocationName(event.locationId)}</span>
                        </div>
                      )}
                    </div>
                  </div>
                </div>
              ))
          )}
        </div>
      </div>

      {/* Approved Suggestions */}
      <div>
        <h3 style={{ display: "flex", alignItems: "center", gap: "0.5rem", marginBottom: "1rem" }}>
          <MapPin size={20} />
          Approved Suggestions ({approvedSuggestions.length})
        </h3>
        <div>
          {approvedSuggestions.length === 0 ? (
            <div className="empty-state">
              <div className="empty-state-icon">
                <MapPin size={32} />
              </div>
              <h3>No approved suggestions yet</h3>
              <p>Approve suggestions from the Suggestions tab to see them here!</p>
            </div>
          ) : (
            approvedSuggestions.map((suggestion) => (
              <div
                key={suggestion.id}
                draggable
                onDragStart={(e) =>
                  handleDragStart(e, suggestion, "suggestion")
                }
                className="list-item"
                style={{ cursor: "move" }}
              >
                <div className="list-item-content">
                  <div className="list-item-title">{suggestion.name}</div>
                  <div className="list-item-subtitle">{suggestion.description}</div>
                  <div style={{ display: "flex", alignItems: "center", gap: "1rem", marginTop: "0.5rem", fontSize: "0.875rem", opacity: 0.7 }}>
                    <div style={{ display: "flex", alignItems: "center", gap: "0.25rem" }}>
                      <MapPin size={14} />
                      <span>{suggestion.location.name}</span>
                    </div>
                    <div style={{ display: "flex", alignItems: "center", gap: "0.25rem" }}>
                      <User size={14} />
                      <span>Suggested by {suggestion.suggestedBy}</span>
                    </div>
                  </div>
                  <div style={{ marginTop: "0.5rem" }}>
                    <span style={{ 
                      display: "inline-block", 
                      padding: "0.25rem 0.5rem", 
                      background: "rgba(0, 0, 0, 0.05)", 
                      borderRadius: "20px", 
                      fontSize: "0.75rem", 
                      fontWeight: "500" 
                    }}>
                      Drag to calendar to schedule
                    </span>
                  </div>
                </div>
              </div>
            ))
          )}
        </div>
      </div>
    </div>
  );
}

