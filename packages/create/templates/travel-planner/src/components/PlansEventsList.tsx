import React from "react";
import { useTripStore } from "../stores/tripStore";
import { CalendarEvent, PlaceSuggestion } from "../types/travel";
import { Calendar, MapPin, Clock, User } from "lucide-react";
import styles from "./PlansEventsList.module.css";

interface PlansEventsListProps {
  currentUser: string;
}

export function PlansEventsList({}: PlansEventsListProps) {
  const { currentTrip } = useTripStore();

  if (!currentTrip) return <div>No trip selected</div>;

  const events = currentTrip.events || [];
  const allApprovedSuggestions = (currentTrip.suggestions || []).filter(
    (s) => s.status === "approved",
  );

  // Filter out suggestions that have already been scheduled as events
  const approvedSuggestions = allApprovedSuggestions.filter((suggestion) => {
    return !events.some(
      (event) =>
        event.title === suggestion.name &&
        event.locationId === suggestion.location.id,
    );
  });

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
      <div className={styles.plansSection}>
        <h3 className={styles.sectionHeader}>
          <Calendar size={20} />
          Scheduled Events ({events.length})
        </h3>
        <div>
          {events.length === 0 ? (
            <div className={styles.emptyState}>
              <div className={styles.emptyStateIcon}>
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
                  className={`list-item ${styles.eventItem}`}
                >
                  <div className="list-item-content">
                    <div className="list-item-title">{event.title}</div>
                    {event.description && (
                      <div className="list-item-subtitle">
                        {event.description}
                      </div>
                    )}
                    <div className={styles.eventMeta}>
                      <div className={styles.eventMetaItem}>
                        <Clock size={14} />
                        <span>
                          {new Date(event.startDate).toLocaleDateString()} -{" "}
                          {new Date(event.endDate).toLocaleDateString()}
                        </span>
                      </div>
                      {getLocationName(event.locationId) && (
                        <div className={styles.eventMetaItem}>
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
        <h3 className={styles.sectionHeader}>
          <MapPin size={20} />
          Approved Suggestions ({approvedSuggestions.length})
        </h3>
        <div>
          {approvedSuggestions.length === 0 ? (
            <div className={styles.emptyState}>
              <div className={styles.emptyStateIcon}>
                <MapPin size={32} />
              </div>
              <h3>No approved suggestions yet</h3>
              <p>
                Approve suggestions from the Suggestions tab to see them here!
              </p>
            </div>
          ) : (
            approvedSuggestions.map((suggestion) => (
              <div
                key={suggestion.id}
                draggable
                onDragStart={(e) =>
                  handleDragStart(e, suggestion, "suggestion")
                }
                className={`list-item ${styles.suggestionItem}`}
              >
                <div className="list-item-content">
                  <div className="list-item-title">{suggestion.name}</div>
                  <div className="list-item-subtitle">
                    {suggestion.description}
                  </div>
                  <div className={styles.suggestionMeta}>
                    <div className={styles.suggestionMetaItem}>
                      <MapPin size={14} />
                      <span>{suggestion.location.name}</span>
                    </div>
                    <div className={styles.suggestionMetaItem}>
                      <User size={14} />
                      <span>Suggested by {suggestion.suggestedBy}</span>
                    </div>
                  </div>
                  <div className={styles.dragHint}>
                    <span className={styles.dragHintBadge}>
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
