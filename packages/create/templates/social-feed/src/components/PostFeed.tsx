import React from "react";
import { usePostsStore } from "../stores/postsStore";
import { PostItem } from "./PostItem";

export const PostFeed: React.FC = () => {
  const { posts } = usePostsStore();

  if (posts.length === 0) {
    return (
      <div className="empty-state">
        <div className="empty-state-icon">📱</div>
        <h3>No posts yet</h3>
        <p>Share something with your friends and family!</p>
      </div>
    );
  }

  return (
    <div>
      {posts.map((post) => (
        <PostItem key={post.id} post={post} />
      ))}
    </div>
  );
};
