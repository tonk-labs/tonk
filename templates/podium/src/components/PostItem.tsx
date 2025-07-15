import React, { useState, useEffect } from "react";
import { Post, TextContent, ImageContent, LinkContent } from "../types/posts";
import { usePostsStore } from "../stores/postsStore";
import { ContentStorage } from "../utils/contentStorage";

interface PostItemProps {
  post: Post;
}

export const PostItem: React.FC<PostItemProps> = ({ post }) => {
  const { deletePost } = usePostsStore();
  const [textContent, setTextContent] = useState<TextContent | null>(null);
  const [imageContent, setImageContent] = useState<ImageContent | null>(null);
  const [linkContent, setLinkContent] = useState<LinkContent | null>(null);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    const loadContent = async () => {
      try {
        setLoading(true);
        switch (post.type) {
          case 'text':
            const textData = await ContentStorage.loadTextContent(post.contentRef);
            setTextContent(textData);
            break;
          case 'image':
            const imageData = await ContentStorage.loadImageContent(post.contentRef);
            setImageContent(imageData);
            break;
          case 'link':
            const linkData = await ContentStorage.loadLinkContent(post.contentRef);
            setLinkContent(linkData);
            break;
        }
      } catch (error) {
        console.error('Error loading content:', error);
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

  if (loading) {
    return (
      <div className="bg-white rounded-lg shadow-md p-6 mb-4">
        <div className="animate-pulse">
          <div className="flex justify-between items-start mb-4">
            <div>
              <div className="h-4 bg-gray-200 rounded w-24 mb-2"></div>
              <div className="h-3 bg-gray-200 rounded w-16"></div>
            </div>
          </div>
          <div className="h-20 bg-gray-200 rounded"></div>
        </div>
      </div>
    );
  }

  return (
    <div className="bg-white rounded-lg shadow-md p-6 mb-4">
      {/* Post header */}
      <div className="flex justify-between items-start mb-4">
        <div>
          <h3 className="font-semibold text-gray-900">{post.author}</h3>
          <p className="text-sm text-gray-500">
            {formatTimestamp(post.timestamp)}
          </p>
        </div>
        <button
          onClick={handleDelete}
          className="text-gray-400 hover:text-red-500 text-sm"
        >
          Delete
        </button>
      </div>

      {/* Post content based on type */}
      {post.type === "text" && textContent && (
        <div className="text-gray-800">
          <p className="whitespace-pre-wrap">{textContent.text}</p>
        </div>
      )}

      {post.type === "image" && imageContent && (
        <div>
          <img
            src={imageContent.imageData}
            alt="Posted image"
            className="w-full max-w-lg rounded-lg mb-2"
          />
          {imageContent.caption && (
            <p className="text-gray-700">{imageContent.caption}</p>
          )}
        </div>
      )}

      {post.type === "link" && linkContent && (
        <div className="border border-gray-200 rounded-lg p-4 hover:bg-gray-50">
          <a
            href={linkContent.url}
            target="_blank"
            rel="noopener noreferrer"
            className="block"
          >
            <div className="flex items-start space-x-3">
              <div className="flex-shrink-0">
                <div className="w-12 h-12 bg-blue-100 rounded-lg flex items-center justify-center">
                  <svg
                    className="w-6 h-6 text-blue-600"
                    fill="none"
                    stroke="currentColor"
                    viewBox="0 0 24 24"
                  >
                    <path
                      strokeLinecap="round"
                      strokeLinejoin="round"
                      strokeWidth={2}
                      d="M13.828 10.172a4 4 0 00-5.656 0l-4 4a4 4 0 105.656 5.656l1.102-1.101m-.758-4.899a4 4 0 005.656 0l4-4a4 4 0 00-5.656-5.656l-1.1 1.1"
                    />
                  </svg>
                </div>
              </div>
              <div className="flex-1 min-w-0">
                {linkContent.title && (
                  <h4 className="text-lg font-medium text-gray-900 mb-1">
                    {linkContent.title}
                  </h4>
                )}
                <p className="text-sm text-blue-600 hover:text-blue-800 truncate">
                  {linkContent.url}
                </p>
                {linkContent.description && (
                  <p className="text-sm text-gray-600 mt-1">
                    {linkContent.description}
                  </p>
                )}
              </div>
            </div>
          </a>
        </div>
      )}

      {/* Post type indicator */}
      <div className="mt-4 flex items-center justify-between">
        <span
          className={`inline-flex items-center px-2.5 py-0.5 rounded-full text-xs font-medium ${
            post.type === "text"
              ? "bg-green-100 text-green-800"
              : post.type === "image"
                ? "bg-purple-100 text-purple-800"
                : "bg-blue-100 text-blue-800"
          }`}
        >
          {post.type === "text" && "📝 Text"}
          {post.type === "image" && "🖼️ Image"}
          {post.type === "link" && "🔗 Link"}
        </span>
      </div>
    </div>
  );
};

