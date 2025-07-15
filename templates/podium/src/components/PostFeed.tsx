import React from "react";
import { usePostsStore } from "../stores/postsStore";
import { PostItem } from "./PostItem";

export const PostFeed: React.FC = () => {
  const { posts } = usePostsStore();

  if (posts.length === 0) {
    return (
      <div className="text-center py-12">
        <div className="text-gray-400 text-6xl mb-4">📱</div>
        <h3 className="text-lg font-medium text-gray-900 mb-2">No posts yet</h3>
        <p className="text-gray-500">
          Be the first to share something with your friends and family!
        </p>
      </div>
    );
  }

  return (
    <div className="space-y-4">
      {posts.map((post) => (
        <PostItem key={post.id} post={post} />
      ))}
    </div>
  );
};

