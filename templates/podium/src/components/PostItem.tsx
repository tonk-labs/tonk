import React, { useState, useEffect } from "react";
import { Post, TextContent, ImageContent, LinkContent } from "../types/posts";
import { usePostsStore } from "../stores/postsStore";
import { useSyncedUsersStore, useLocalAuthStore } from "../stores/userStore";
import { ContentStorage } from "../utils/contentStorage";
import { CommentSection } from "./CommentSection";

interface PostItemProps {
  post: Post;
}

export const PostItem: React.FC<PostItemProps> = ({ post }) => {
  const { deletePost } = usePostsStore();
  const { getUser, isUserOwner } = useSyncedUsersStore();
  const { currentUser } = useLocalAuthStore();
  const [textContent, setTextContent] = useState<TextContent | null>(null);
  const [imageContent, setImageContent] = useState<ImageContent | null>(null);
  const [linkContent, setLinkContent] = useState<LinkContent | null>(null);
  const [loading, setLoading] = useState(true);

  const author = getUser(post.authorId);

  useEffect(() => {
    const loadContent = async () => {
      try {
        setLoading(true);
        switch (post.type) {
          case "text":
            const textData = await ContentStorage.loadTextContent(
              post.contentRef,
            );
            setTextContent(textData);
            break;
          case "image":
            const imageData = await ContentStorage.loadImageContent(
              post.contentRef,
            );
            setImageContent(imageData);
            break;
          case "link":
            const linkData = await ContentStorage.loadLinkContent(
              post.contentRef,
            );
            setLinkContent(linkData);
            break;
        }
      } catch (error) {
        console.error("Error loading content:", error);
      } finally {
        setLoading(false);
      }
    };

    loadContent();
  }, [post.contentRef, post.type]);

  const formatTimestamp = (timestamp: number) => {
    const date = new Date(timestamp);
    const now = new Date();
    const diffInHours = (now.getTime() - date.getTime()) / (1000 * 60 * 60);

    if (diffInHours < 1) {
      const diffInMinutes = Math.floor(diffInHours * 60);
      return `${diffInMinutes}m ago`;
    } else if (diffInHours < 24) {
      return `${Math.floor(diffInHours)}h ago`;
    } else {
      return date.toLocaleDateString();
    }
  };

  const handleDelete = () => {
    if (window.confirm("Are you sure you want to delete this post?")) {
      deletePost(post.id);
    }
  };

  const canDelete =
    currentUser &&
    (currentUser.id === post.authorId || isUserOwner(currentUser.id));

  if (loading) {
    return (
      <div className="card">
        <div className="post-header">
          <div>
            <div className="loading-skeleton loading-title"></div>
            <div className="loading-skeleton loading-subtitle"></div>
          </div>
        </div>
        <div className="loading-skeleton loading-content"></div>
      </div>
    );
  }

  return (
    <div className="card post-item">
      {/* Post header */}
      <div className="post-header">
        <div className="post-author">
          {author?.profilePicture && (
            <img
              src={author.profilePicture}
              alt={author.name}
              className="post-avatar"
            />
          )}
          <div className="post-author-info">
            <div
              style={{ display: "flex", alignItems: "center", gap: "0.5rem" }}
            >
              <h3>{author?.name || "Unknown User"}</h3>
              {author?.isOwner && <span className="owner-badge">Owner</span>}
            </div>
            <p className="post-timestamp">{formatTimestamp(post.timestamp)}</p>
          </div>
        </div>
        {canDelete && (
          <button onClick={handleDelete} className="btn-danger">
            Delete
          </button>
        )}
      </div>

      {/* Post content based on type */}
      {post.type === "text" && textContent && (
        <div className="post-content">
          <p style={{ whiteSpace: "pre-wrap" }}>{textContent.text}</p>
        </div>
      )}

      {post.type === "image" && imageContent && (
        <div className="post-content">
          <img
            src={imageContent.imageData}
            alt="Posted image"
            className="post-image"
          />
          {imageContent.caption && <p>{imageContent.caption}</p>}
        </div>
      )}

      {post.type === "link" && linkContent && (
        <div className="post-content">
          <a
            href={linkContent.url}
            target="_blank"
            rel="noopener noreferrer"
            className="post-link"
          >
            <div className="post-link-content">
              <div className="post-link-icon">
                <svg
                  width="24"
                  height="24"
                  fill="none"
                  stroke="currentColor"
                  viewBox="0 0 24 24"
                  style={{ opacity: 0.6 }}
                >
                  <path
                    strokeLinecap="round"
                    strokeLinejoin="round"
                    strokeWidth={2}
                    d="M13.828 10.172a4 4 0 00-5.656 0l-4 4a4 4 0 105.656 5.656l1.102-1.101m-.758-4.899a4 4 0 005.656 0l4-4a4 4 0 00-5.656-5.656l-1.1 1.1"
                  />
                </svg>
              </div>
              <div className="post-link-info">
                {linkContent.title && <h4>{linkContent.title}</h4>}
                <p className="post-link-url">{linkContent.url}</p>
                {linkContent.description && (
                  <p className="post-link-description">
                    {linkContent.description}
                  </p>
                )}
              </div>
            </div>
          </a>
        </div>
      )}

      {/* Comments section */}
      <CommentSection postId={post.id} />
    </div>
  );
};
