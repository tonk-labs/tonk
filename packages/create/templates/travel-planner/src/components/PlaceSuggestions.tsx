import React, { useState } from "react";
import { useTripStore } from "../stores/tripStore";
import { PlaceSuggestion } from "../types/travel";
import { Plus, ThumbsUp, Check, X, MapPin, User } from "lucide-react";
import PlaceSearch from "./PlaceSearch";

interface PlaceSuggestionsProps {
  currentUser: string;
}

export function PlaceSuggestions({ currentUser }: PlaceSuggestionsProps) {
  const {
    currentTrip,
    addSuggestion,
    voteSuggestion,
    approveSuggestion,
    rejectSuggestion,
    addLocation,
  } = useTripStore();

  const [showSuggestionForm, setShowSuggestionForm] = useState(false);
  const [activeTab, setActiveTab] = useState<
    "pending" | "approved" | "rejected"
  >("pending");
  const [suggestionForm, setSuggestionForm] = useState({
    name: "",
    description: "",
    locationName: "",
    locationAddress: "",
    locationLatitude: 0,
    locationLongitude: 0,
    locationPlaceId: "",
    locationCategory: "attraction" as const,
  });

  if (!currentTrip) return <div className="card">No trip selected</div>;

  const suggestions = currentTrip.suggestions || [];
  const members = currentTrip.members || [];

  const pendingSuggestions = suggestions.filter((s) => s.status === "pending");
  const approvedSuggestions = suggestions.filter(
    (s) => s.status === "approved",
  );
  const rejectedSuggestions = suggestions.filter(
    (s) => s.status === "rejected",
  );

  const handleSubmitSuggestion = (e: React.FormEvent) => {
    e.preventDefault();

    // Validate that a location has been selected
    if (!suggestionForm.locationName || suggestionForm.locationLatitude === 0) {
      alert("Please select a location from the search results");
      return;
    }

    // Create location data for the suggestion
    const location = {
      id: Math.random().toString(36).substring(2, 11),
      name: suggestionForm.locationName,
      address: suggestionForm.locationAddress,
      latitude: suggestionForm.locationLatitude,
      longitude: suggestionForm.locationLongitude,
      category: suggestionForm.locationCategory,
      placeId: suggestionForm.locationPlaceId || null,
      description: null,
      addedBy: currentUser,
      addedAt: new Date().toISOString(),
    };

    addSuggestion({
      name: suggestionForm.name,
      description: suggestionForm.description,
      location,
      suggestedBy: currentUser,
    });

    setShowSuggestionForm(false);
    setSuggestionForm({
      name: "",
      description: "",
      locationName: "",
      locationAddress: "",
      locationLatitude: 0,
      locationLongitude: 0,
      locationPlaceId: "",
      locationCategory: "attraction",
    });
  };

  const handleVote = (suggestionId: string) => {
    voteSuggestion(suggestionId, currentUser);
  };

  const handleApprove = (suggestion: PlaceSuggestion) => {
    approveSuggestion(suggestion.id);
    // Also add the location to the trip
    addLocation({
      name: suggestion.location.name,
      address: suggestion.location.address,
      latitude: suggestion.location.latitude,
      longitude: suggestion.location.longitude,
      category: suggestion.location.category,
      placeId: suggestion.location.placeId || null,
      description: suggestion.description || null,
      addedBy: currentUser,
    });
  };

  const getMemberName = (memberId: string) => {
    const member = members.find(
      (m) => m.id === memberId || m.name === memberId,
    );
    return member?.name || memberId;
  };

  const SuggestionCard = ({ suggestion }: { suggestion: PlaceSuggestion }) => {
    const hasVoted = suggestion.votes.includes(currentUser);
    const voteCount = suggestion.votes.length;

    return (
      <div className="suggestion-card">
        <div className="suggestion-header">
          <div>
            <h3 className="suggestion-title">{suggestion.name}</h3>
            <div className="suggestion-meta">
              <User size={14} />
              <span>Suggested by {getMemberName(suggestion.suggestedBy)}</span>
              <span className="suggestion-meta-separator">•</span>
              <span>
                {new Date(suggestion.suggestedAt).toLocaleDateString()}
              </span>
            </div>
          </div>
          <div className="suggestion-actions">
            {suggestion.status === "pending" && (
              <>
                <button
                  onClick={() => handleVote(suggestion.id)}
                  className={`suggestion-vote-btn ${hasVoted ? "voted" : ""}`}
                >
                  <ThumbsUp size={14} />
                  <span>{voteCount}</span>
                </button>
                <button
                  onClick={() => handleApprove(suggestion)}
                  className="icon-btn"
                  style={{ color: "#16a34a" }}
                  title="Approve suggestion"
                >
                  <Check size={16} />
                </button>
                <button
                  onClick={() => rejectSuggestion(suggestion.id)}
                  className="icon-btn"
                  style={{ color: "#dc2626" }}
                  title="Reject suggestion"
                >
                  <X size={16} />
                </button>
              </>
            )}
            {suggestion.status === "approved" && (
              <span className="suggestion-status-badge approved">Approved</span>
            )}
            {suggestion.status === "rejected" && (
              <span className="suggestion-status-badge rejected">Rejected</span>
            )}
          </div>
        </div>

        <p className="suggestion-description">{suggestion.description}</p>

        <div className="suggestion-location">
          <MapPin size={14} />
          <div className="suggestion-location-details">
            <div className="suggestion-location-name">
              {suggestion.location.name}
            </div>
            <div className="suggestion-location-address">
              {suggestion.location.address}
            </div>
            <div className="suggestion-location-category">
              {suggestion.location.category}
            </div>
          </div>
        </div>
      </div>
    );
  };

  const getCurrentSuggestions = () => {
    switch (activeTab) {
      case "pending":
        return pendingSuggestions;
      case "approved":
        return approvedSuggestions;
      case "rejected":
        return rejectedSuggestions;
      default:
        return pendingSuggestions;
    }
  };

  const getEmptyStateContent = () => {
    switch (activeTab) {
      case "pending":
        return {
          icon: "💡",
          title: "No pending suggestions yet",
          description: "Be the first to suggest a place to visit!",
        };
      case "approved":
        return {
          icon: "✅",
          title: "No approved suggestions yet",
          description: "Approved suggestions will appear here.",
        };
      case "rejected":
        return {
          icon: "❌",
          title: "No rejected suggestions",
          description: "Rejected suggestions will appear here.",
        };
      default:
        return {
          icon: "💡",
          title: "No suggestions yet",
          description: "Start by adding a suggestion!",
        };
    }
  };

  return (
    <div className="card">
      <div
        className="list-item"
        style={{
          marginBottom: 0,
          padding: "1rem 0",
          background: "transparent",
        }}
      >
        <h2>Place Suggestions</h2>
        <button
          onClick={() => setShowSuggestionForm(true)}
          className="btn btn-primary"
          style={{ display: "flex", alignItems: "center", gap: "0.5rem" }}
        >
          <Plus size={20} />
          Suggest Place
        </button>
      </div>

      {/* Suggestion Form Modal */}
      {showSuggestionForm && (
        <div className="modal-overlay">
          <div
            className="modal-content"
            style={{ maxHeight: "90vh", overflowY: "auto" }}
          >
            <h3 className="modal-title">Suggest a New Place</h3>

            <form onSubmit={handleSubmitSuggestion}>
              <div className="form-group">
                <label className="form-label">Suggestion Title</label>
                <input
                  type="text"
                  value={suggestionForm.name}
                  onChange={(e) =>
                    setSuggestionForm({
                      ...suggestionForm,
                      name: e.target.value,
                    })
                  }
                  className="form-input"
                  placeholder="e.g., Visit the Golden Gate Bridge"
                  required
                />
              </div>

              <div className="form-group">
                <label className="form-label">Description</label>
                <textarea
                  value={suggestionForm.description}
                  onChange={(e) =>
                    setSuggestionForm({
                      ...suggestionForm,
                      description: e.target.value,
                    })
                  }
                  className="form-textarea"
                  rows={3}
                  placeholder="Why should we visit this place?"
                  required
                />
              </div>

              <div className="form-group">
                <label className="form-label">Location</label>
                <PlaceSearch
                  onPlaceSelect={(
                    latitude,
                    longitude,
                    name,
                    address,
                    placeId,
                  ) => {
                    setSuggestionForm({
                      ...suggestionForm,
                      locationName: name,
                      locationAddress: address,
                      locationLatitude: latitude,
                      locationLongitude: longitude,
                      locationPlaceId: placeId || "",
                    });
                  }}
                  placeholder="Search for a location to suggest"
                />
                {suggestionForm.locationName && (
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
                      {suggestionForm.locationName}
                    </div>
                    <div style={{ opacity: 0.7 }}>
                      {suggestionForm.locationAddress}
                    </div>
                  </div>
                )}
              </div>

              <div className="form-group">
                <label className="form-label">Category</label>
                <select
                  value={suggestionForm.locationCategory}
                  onChange={(e) =>
                    setSuggestionForm({
                      ...suggestionForm,
                      locationCategory: e.target.value as any,
                    })
                  }
                  className="form-select"
                >
                  <option value="attraction">Attraction</option>
                  <option value="restaurant">Restaurant</option>
                  <option value="hotel">Hotel</option>
                  <option value="activity">Activity</option>
                  <option value="transport">Transport</option>
                  <option value="other">Other</option>
                </select>
              </div>

              <div className="modal-actions">
                <button
                  type="button"
                  onClick={() => {
                    setShowSuggestionForm(false);
                    setSuggestionForm({
                      name: "",
                      description: "",
                      locationName: "",
                      locationAddress: "",
                      locationLatitude: 0,
                      locationLongitude: 0,
                      locationPlaceId: "",
                      locationCategory: "attraction",
                    });
                  }}
                  className="btn btn-secondary"
                >
                  Cancel
                </button>
                <button type="submit" className="btn btn-primary">
                  Add Suggestion
                </button>
              </div>
            </form>
          </div>
        </div>
      )}

      {/* Suggestions Tabs */}
      <div className="suggestions-tabs">
        <nav className="suggestions-tabs-nav">
          <button
            onClick={() => setActiveTab("pending")}
            className={`suggestions-tab ${activeTab === "pending" ? "active" : ""}`}
          >
            Pending ({pendingSuggestions.length})
          </button>
          {approvedSuggestions.length > 0 && (
            <button
              onClick={() => setActiveTab("approved")}
              className={`suggestions-tab ${activeTab === "approved" ? "active" : ""}`}
            >
              Approved ({approvedSuggestions.length})
            </button>
          )}
          {rejectedSuggestions.length > 0 && (
            <button
              onClick={() => setActiveTab("rejected")}
              className={`suggestions-tab ${activeTab === "rejected" ? "active" : ""}`}
            >
              Rejected ({rejectedSuggestions.length})
            </button>
          )}
        </nav>
      </div>

      {/* Current Tab Content */}
      <div className="suggestions-section">
        {getCurrentSuggestions().length === 0 ? (
          <div className="empty-state">
            <div className="empty-state-icon">
              {getEmptyStateContent().icon}
            </div>
            <h3>{getEmptyStateContent().title}</h3>
            <p>{getEmptyStateContent().description}</p>
          </div>
        ) : (
          getCurrentSuggestions().map((suggestion) => (
            <SuggestionCard key={suggestion.id} suggestion={suggestion} />
          ))
        )}
      </div>
    </div>
  );
}

