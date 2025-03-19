import React, { useState } from "react";
import { Users } from "lucide-react";
import { useLocationStore, useUserStore } from "../stores";

// Define what "in common" means - locations within this distance (meters) are considered common
const COMMON_LOCATION_THRESHOLD = 100; // meters

interface UserComparisonProps {
  onShowCommonLocations: (commonLocations: string[]) => void;
}

const UserComparison: React.FC<UserComparisonProps> = ({
  onShowCommonLocations,
}) => {
  const { locations } = useLocationStore();
  const userNames = useLocationStore((state) => state.userNames);
  const { profile: currentUser } = useUserStore();
  const [selectedUserId, setSelectedUserId] = useState<string>("");

  // Get unique user IDs from locations
  const userIds = Object.values(locations).reduce((acc, location) => {
    if (
      !acc.includes(location.addedBy) &&
      location.addedBy !== currentUser.id
    ) {
      acc.push(location.addedBy);
    }
    return acc;
  }, [] as string[]);

  // Calculate distance between two points in meters using Haversine formula
  const calculateDistance = (
    lat1: number,
    lon1: number,
    lat2: number,
    lon2: number,
  ): number => {
    const R = 6371e3; // Earth radius in meters
    const φ1 = (lat1 * Math.PI) / 180;
    const φ2 = (lat2 * Math.PI) / 180;
    const Δφ = ((lat2 - lat1) * Math.PI) / 180;
    const Δλ = ((lon2 - lon1) * Math.PI) / 180;

    const a =
      Math.sin(Δφ / 2) * Math.sin(Δφ / 2) +
      Math.cos(φ1) * Math.cos(φ2) * Math.sin(Δλ / 2) * Math.sin(Δλ / 2);
    const c = 2 * Math.atan2(Math.sqrt(a), Math.sqrt(1 - a));
    return R * c;
  };

  // Find common locations between current user and selected user
  const findCommonLocations = () => {
    if (!selectedUserId) return;

    const currentUserLocations = Object.values(locations).filter(
      (loc) => loc.addedBy === currentUser.id,
    );
    const selectedUserLocations = Object.values(locations).filter(
      (loc) => loc.addedBy === selectedUserId,
    );

    // Find locations that are close to each other (within threshold)
    const commonLocationIds: string[] = [];

    currentUserLocations.forEach((userLoc) => {
      selectedUserLocations.forEach((otherLoc) => {
        const distance = calculateDistance(
          userLoc.latitude,
          userLoc.longitude,
          otherLoc.latitude,
          otherLoc.longitude,
        );

        if (distance <= COMMON_LOCATION_THRESHOLD) {
          // Consider these locations as "common"
          commonLocationIds.push(userLoc.id, otherLoc.id);
        }
      });
    });

    // Use Set to remove duplicates
    const uniqueCommonLocationIds = [...new Set(commonLocationIds)];
    onShowCommonLocations(uniqueCommonLocationIds);
  };

  return (
    <div className="mb-6 p-4 bg-gray-50 rounded-lg">
      <h3 className="font-semibold mb-3 flex items-center gap-2">
        <Users className="h-4 w-4" />
        Compare Places
      </h3>

      {userIds.length === 0 ? (
        <p className="text-sm text-gray-500">
          No other users have added locations yet
        </p>
      ) : (
        <>
          <div className="mb-3">
            <label
              htmlFor="user-select"
              className="text-sm font-medium block mb-1"
            >
              Select user to compare with:
            </label>
            <select
              id="user-select"
              value={selectedUserId}
              onChange={(e) => setSelectedUserId(e.target.value)}
              className="w-full px-3 py-2 border rounded-md"
            >
              <option value="">Select a user</option>
              {userIds.map((userId) => (
                <option key={userId} value={userId}>
                  {userNames[userId] || "Anonymous"}
                </option>
              ))}
            </select>
          </div>
          <button
            onClick={findCommonLocations}
            disabled={!selectedUserId}
            className={`w-full py-2 rounded-md text-white text-sm ${
              selectedUserId
                ? "bg-blue-500 hover:bg-blue-600"
                : "bg-gray-300 cursor-not-allowed"
            }`}
          >
            Show Common Places
          </button>
        </>
      )}
    </div>
  );
};

export default UserComparison;
