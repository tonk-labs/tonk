import React, { useState, useEffect } from "react";
import { useTripStore } from "../stores/tripStore";
import { Calendar } from "../components/Calendar";
import { PlaceSuggestions } from "../components/PlaceSuggestions";
import { SharedNotes } from "../components/SharedNotes";
import { PlansEventsList } from "../components/PlansEventsList";
import MapView from "../components/MapView";
import {
  Calendar as CalendarIcon,
  Lightbulb,
  FileText,
  Users,
  Settings,
  Plus,
  Map,
} from "lucide-react";

type TabType = "calendar" | "suggestions" | "notes" | "members" | "map";

export default function TravelPlanner() {
  const { currentTrip, createTrip, addMember, loadTrip, isLoading } =
    useTripStore();
  const [activeTab, setActiveTab] = useState<TabType>("calendar");
  const [currentUser, setCurrentUser] = useState<string>("");
  const [showTripForm, setShowTripForm] = useState(false);
  const [showSettings, setShowSettings] = useState(false);
  const [showUserSelection, setShowUserSelection] = useState(false);
  const [newUserName, setNewUserName] = useState("");
  const [selectedExistingUser, setSelectedExistingUser] = useState("");
  const [tripForm, setTripForm] = useState({
    name: "",
    description: "",
    startDate: "",
    endDate: "",
  });

  useEffect(() => {
    // Load current user from localStorage
    const savedUser = localStorage.getItem("travel-planner-current-user");
    if (savedUser) {
      setCurrentUser(savedUser);
    }

    // Try to load existing trip data
    loadTrip();
  }, [loadTrip]);

  useEffect(() => {
    // Show appropriate modal based on state
    if (!isLoading) {
      // Check localStorage again to ensure we have the most current user state
      const savedUser = localStorage.getItem("travel-planner-current-user");
      const userToCheck = currentUser || savedUser;

      if (!currentTrip) {
        setShowTripForm(true);
      } else if (!userToCheck) {
        // If we have a trip but no current user, show user selection
        setShowUserSelection(true);
      }
    }
  }, [currentTrip, currentUser, isLoading]);

  const handleCreateTrip = (e: React.FormEvent) => {
    e.preventDefault();
    createTrip(
      tripForm.name,
      tripForm.description,
      new Date(tripForm.startDate),
      new Date(tripForm.endDate),
      currentUser,
    );
    setShowTripForm(false);
    setTripForm({
      name: "",
      description: "",
      startDate: "",
      endDate: "",
    });
  };

  const handleUserSelection = () => {
    let userName = "";

    if (selectedExistingUser) {
      userName = selectedExistingUser;
    } else if (newUserName.trim()) {
      userName = newUserName.trim();
      // Add new user to trip members
      addMember({
        id: Math.random().toString(36).substring(2, 11),
        name: userName,
      });
    }

    if (userName) {
      setCurrentUser(userName);
      localStorage.setItem("travel-planner-current-user", userName);
      setShowUserSelection(false);
      setNewUserName("");
      setSelectedExistingUser("");
    }
  };

  const tabs = [
    { id: "calendar" as const, label: "Calendar", icon: CalendarIcon },
    { id: "suggestions" as const, label: "Suggestions", icon: Lightbulb },
    { id: "notes" as const, label: "Notes", icon: FileText },
    { id: "members" as const, label: "Members", icon: Users },
    { id: "map" as const, label: "Map", icon: Map },
  ];

  const renderTabContent = () => {
    if (!currentTrip) return null;

    switch (activeTab) {
      case "calendar":
        return (
          <div className="grid grid-cols-1 grid-cols-2">
            <PlansEventsList currentUser={currentUser} />
            <Calendar currentUser={currentUser} />
          </div>
        );
      case "suggestions":
        return <PlaceSuggestions currentUser={currentUser} />;
      case "notes":
        return <SharedNotes currentUser={currentUser} />;
      case "members":
        return <MembersTab currentUser={currentUser} />;
      case "map":
        return <MapView currentUser={currentUser} />;
      default:
        return null;
    }
  };

  // Show loading screen while sync is initializing
  if (isLoading) {
    return (
      <div className="app-container">
        <div
          style={{
            display: "flex",
            justifyContent: "center",
            alignItems: "center",
            height: "100vh",
            flexDirection: "column",
            gap: "1rem",
          }}
        >
          <div>Loading travel planner...</div>
          <div style={{ fontSize: "0.875rem", opacity: 0.7 }}>
            Syncing your data...
          </div>
        </div>
      </div>
    );
  }

  return (
    <div className="app-container">
      {/* Header */}
      <header className="header">
        <div className="header-content">
          <div className="header-title">
            <h1>Travel Planner</h1>
            {currentTrip && (
              <div className="header-trip-info">
                <h2>{currentTrip.name}</h2>
                <p>
                  {new Date(currentTrip.startDate).toLocaleDateString()} -{" "}
                  {new Date(currentTrip.endDate).toLocaleDateString()}
                </p>
              </div>
            )}
          </div>
          <div className="header-user">
            <span>Welcome, {currentUser || "Guest"}</span>
            <button className="icon-btn" onClick={() => setShowSettings(true)}>
              <Settings size={20} />
            </button>
          </div>
        </div>
      </header>

      {/* User Selection Modal */}
      {showUserSelection && (
        <div className="modal-overlay">
          <div className="modal-content">
            <h3 className="modal-title">Who are you?</h3>

            <div className="form-group">
              <label className="form-label">Select existing member</label>
              <select
                value={selectedExistingUser}
                onChange={(e) => setSelectedExistingUser(e.target.value)}
                className="form-select"
              >
                <option value="">-- Select a member --</option>
                {currentTrip?.members.map((member) => (
                  <option key={member.id} value={member.name}>
                    {member.name}
                  </option>
                ))}
              </select>
            </div>

            <div className="form-group">
              <label className="form-label">Or create new user</label>
              <input
                type="text"
                value={newUserName}
                onChange={(e) => setNewUserName(e.target.value)}
                className="form-input"
                placeholder="Enter your name"
              />
            </div>

            <div className="modal-actions">
              <button
                type="button"
                onClick={handleUserSelection}
                className="btn btn-primary"
                disabled={!selectedExistingUser && !newUserName.trim()}
              >
                Continue
              </button>
            </div>
          </div>
        </div>
      )}

      {/* Settings Modal */}
      {showSettings && (
        <div className="modal-overlay">
          <div className="modal-content">
            <h3 className="modal-title">Settings</h3>

            <div className="form-group">
              <label className="form-label">Select User</label>
              <select
                value={currentUser}
                onChange={(e) => {
                  setCurrentUser(e.target.value);
                  localStorage.setItem(
                    "travel-planner-current-user",
                    e.target.value,
                  );
                }}
                className="form-select"
              >
                {currentTrip?.members.map((member) => (
                  <option key={member.id} value={member.name}>
                    {member.name}
                  </option>
                ))}
              </select>
            </div>

            <div className="modal-actions">
              <button
                type="button"
                onClick={() => setShowSettings(false)}
                className="btn btn-secondary"
              >
                Cancel
              </button>
              <button
                type="button"
                onClick={() => setShowSettings(false)}
                className="btn btn-primary"
              >
                Save
              </button>
            </div>
          </div>
        </div>
      )}

      {/* Trip Creation Modal */}
      {showTripForm && (
        <div className="modal-overlay">
          <div className="modal-content">
            <h3 className="modal-title">Create New Trip</h3>

            <form onSubmit={handleCreateTrip}>
              <div className="form-group">
                <label className="form-label">Trip Name</label>
                <input
                  type="text"
                  value={tripForm.name}
                  onChange={(e) =>
                    setTripForm({ ...tripForm, name: e.target.value })
                  }
                  className="form-input"
                  placeholder="e.g., Summer Vacation 2024"
                  required
                />
              </div>

              <div className="form-group">
                <label className="form-label">Description</label>
                <textarea
                  value={tripForm.description}
                  onChange={(e) =>
                    setTripForm({ ...tripForm, description: e.target.value })
                  }
                  className="form-textarea"
                  rows={3}
                  placeholder="Describe your trip..."
                  required
                />
              </div>

              <div className="form-group">
                <label className="form-label">Start Date</label>
                <input
                  type="date"
                  value={tripForm.startDate}
                  onChange={(e) =>
                    setTripForm({ ...tripForm, startDate: e.target.value })
                  }
                  className="form-input"
                  required
                />
              </div>

              <div className="form-group">
                <label className="form-label">End Date</label>
                <input
                  type="date"
                  value={tripForm.endDate}
                  onChange={(e) =>
                    setTripForm({ ...tripForm, endDate: e.target.value })
                  }
                  className="form-input"
                  required
                />
              </div>

              <div className="modal-actions">
                <button type="submit" className="btn btn-primary">
                  Create Trip
                </button>
              </div>
            </form>
          </div>
        </div>
      )}

      {currentTrip && (
        <>
          {/* Navigation Tabs */}
          <nav className="nav-tabs">
            <div className="nav-tabs-content">
              {tabs.map((tab) => {
                const Icon = tab.icon;
                return (
                  <button
                    key={tab.id}
                    onClick={() => setActiveTab(tab.id)}
                    className={`nav-tab ${activeTab === tab.id ? "active" : ""}`}
                  >
                    <Icon size={18} />
                    {tab.label}
                  </button>
                );
              })}
            </div>
          </nav>

          {/* Main Content */}
          <main className="main-content">{renderTabContent()}</main>
        </>
      )}
    </div>
  );
}

// Members Tab Component
function MembersTab({}: { currentUser: string }) {
  const { currentTrip, addMember, removeMember } = useTripStore();
  const [showMemberForm, setShowMemberForm] = useState(false);
  const [memberForm, setMemberForm] = useState({
    name: "",
  });

  if (!currentTrip) return null;

  const members = currentTrip.members || [];

  const handleAddMember = (e: React.FormEvent) => {
    e.preventDefault();
    addMember({
      id: Math.random().toString(36).substring(2, 11),
      name: memberForm.name,
    });
    setShowMemberForm(false);
    setMemberForm({
      name: "",
    });
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
        <h2>Trip Members</h2>
        <button
          onClick={() => setShowMemberForm(true)}
          className="btn btn-primary"
          style={{ display: "flex", alignItems: "center", gap: "0.5rem" }}
        >
          <Plus size={20} />
          Add Member
        </button>
      </div>

      {/* Member Form Modal */}
      {showMemberForm && (
        <div className="modal-overlay">
          <div className="modal-content">
            <h3 className="modal-title">Add New Member</h3>

            <form onSubmit={handleAddMember}>
              <div className="form-group">
                <label className="form-label">Name</label>
                <input
                  type="text"
                  value={memberForm.name}
                  onChange={(e) =>
                    setMemberForm({ ...memberForm, name: e.target.value })
                  }
                  className="form-input"
                  required
                />
              </div>

              <div className="modal-actions">
                <button
                  type="button"
                  onClick={() => setShowMemberForm(false)}
                  className="btn btn-secondary"
                >
                  Cancel
                </button>
                <button type="submit" className="btn btn-primary">
                  Add Member
                </button>
              </div>
            </form>
          </div>
        </div>
      )}

      {/* Members List */}
      <div>
        {members.map((member) => (
          <div key={member.id} className="list-item">
            <div className="list-item-content">
              <div className="list-item-title">{member.name}</div>
              <div className="list-item-meta">
                Joined {new Date(member.joinedAt).toLocaleDateString()}
              </div>
            </div>
            <div className="list-item-actions">
              <button
                onClick={() => removeMember(member.id)}
                className="btn-danger"
              >
                Remove
              </button>
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}
