import React from "react";
import { PostCreator } from "../components/PostCreator";
import { PostFeed } from "../components/PostFeed";
import { UserAuth } from "../components/UserAuth";

export const MediaFeed: React.FC = () => {
  return (
    <div className="app-container">
      <div className="main-content">
        {/* Header */}
        <div className="header">
          <h1>Podium</h1>
          <p className="header-subtitle">
            Share moments with your intimate circle
          </p>
        </div>

        {/* User Authentication */}
        <UserAuth />

        {/* Post Creator */}
        <PostCreator />

        {/* Posts Feed */}
        <PostFeed />
      </div>
    </div>
  );
};
