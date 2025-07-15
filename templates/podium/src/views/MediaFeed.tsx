import React from 'react';
import { PostCreator } from '../components/PostCreator';
import { PostFeed } from '../components/PostFeed';

export const MediaFeed: React.FC = () => {
  return (
    <div className="min-h-screen bg-gray-50">
      <div className="max-w-2xl mx-auto py-8 px-4">
        {/* Header */}
        <div className="text-center mb-8">
          <h1 className="text-3xl font-bold text-gray-900 mb-2">
            Family & Friends Feed
          </h1>
          <p className="text-gray-600">
            Share moments, thoughts, and links with your intimate circle
          </p>
        </div>

        {/* Post Creator */}
        <PostCreator />

        {/* Posts Feed */}
        <PostFeed />
      </div>
    </div>
  );
};