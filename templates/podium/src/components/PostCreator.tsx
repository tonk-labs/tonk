import React, { useState } from "react";
import { usePostsStore } from "../stores/postsStore";
import { PostType } from "../types/posts";
import { ContentStorage } from "../utils/contentStorage";

export const PostCreator: React.FC = () => {
  const [postType, setPostType] = useState<PostType>("text");
  const [content, setContent] = useState("");
  const [url, setUrl] = useState("");
  const [caption, setCaption] = useState("");
  const [author, setAuthor] = useState("Anonymous");
  const [imageFile, setImageFile] = useState<File | null>(null);

  const { addPost } = usePostsStore();

  const extractDomainFromUrl = (url: string): string => {
    try {
      const domain = new URL(url).hostname;
      return domain.replace("www.", "");
    } catch {
      return url;
    }
  };

  const handleUrlChange = (newUrl: string) => {
    setUrl(newUrl);
    if (newUrl.trim() && !content.trim()) {
      const domain = extractDomainFromUrl(newUrl);
      setContent(`Check out this link from ${domain}`);
    }
  };

  const handleImageUpload = (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    if (file) {
      setImageFile(file);
    }
  };

  const convertImageToBase64 = (file: File): Promise<string> => {
    return new Promise((resolve, reject) => {
      const reader = new FileReader();
      reader.onload = () => resolve(reader.result as string);
      reader.onerror = reject;
      reader.readAsDataURL(file);
    });
  };

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();

    if (!author.trim()) {
      alert("Please enter your name");
      return;
    }

    try {
      const contentId = ContentStorage.generateContentId();

      switch (postType) {
        case "text":
          if (!content.trim()) {
            alert("Please enter some text");
            return;
          }
          
          // Save text content to keepsync
          await ContentStorage.saveTextContent(contentId, {
            text: content.trim(),
          });
          
          addPost({
            type: "text",
            author: author.trim(),
            contentRef: contentId,
          });
          break;

        case "image":
          if (!imageFile) {
            alert("Please select an image");
            return;
          }
          
          const imageData = await convertImageToBase64(imageFile);
          
          // Save image content to keepsync
          await ContentStorage.saveImageContent(contentId, {
            imageData,
            caption: caption.trim(),
          });
          
          addPost({
            type: "image",
            author: author.trim(),
            contentRef: contentId,
          });
          break;

        case "link":
          if (!url.trim()) {
            alert("Please enter a URL");
            return;
          }
          
          // Save link content to keepsync
          await ContentStorage.saveLinkContent(contentId, {
            url: url.trim(),
            title: content.trim(),
            description: content.trim(),
          });
          
          addPost({
            type: "link",
            author: author.trim(),
            contentRef: contentId,
          });
          break;
      }

      // Reset form
      setContent("");
      setUrl("");
      setCaption("");
      setImageFile(null);
    } catch (error) {
      console.error("Error creating post:", error);
      alert("Error creating post. Please try again.");
    }
  };

  return (
    <div className="bg-white rounded-lg shadow-md p-6 mb-6">
      <h2 className="text-xl font-semibold mb-4">Create a Post</h2>

      <form onSubmit={handleSubmit}>
        {/* Author input */}
        <div className="mb-4">
          <label className="block text-sm font-medium text-gray-700 mb-2">
            Your Name
          </label>
          <input
            type="text"
            value={author}
            onChange={(e) => setAuthor(e.target.value)}
            className="w-full px-3 py-2 border border-gray-300 rounded-md focus:outline-none focus:ring-2 focus:ring-blue-500"
            placeholder="Enter your name"
          />
        </div>

        {/* Post type selector */}
        <div className="mb-4">
          <label className="block text-sm font-medium text-gray-700 mb-2">
            Post Type
          </label>
          <div className="flex space-x-4">
            <button
              type="button"
              onClick={() => setPostType("text")}
              className={`px-4 py-2 rounded-md ${
                postType === "text"
                  ? "bg-blue-500 text-white"
                  : "bg-gray-200 text-gray-700 hover:bg-gray-300"
              }`}
            >
              Text
            </button>
            <button
              type="button"
              onClick={() => setPostType("image")}
              className={`px-4 py-2 rounded-md ${
                postType === "image"
                  ? "bg-blue-500 text-white"
                  : "bg-gray-200 text-gray-700 hover:bg-gray-300"
              }`}
            >
              Image
            </button>
            <button
              type="button"
              onClick={() => setPostType("link")}
              className={`px-4 py-2 rounded-md ${
                postType === "link"
                  ? "bg-blue-500 text-white"
                  : "bg-gray-200 text-gray-700 hover:bg-gray-300"
              }`}
            >
              Link
            </button>
          </div>
        </div>

        {/* Content based on post type */}
        {postType === "text" && (
          <div className="mb-4">
            <label className="block text-sm font-medium text-gray-700 mb-2">
              Message
            </label>
            <textarea
              value={content}
              onChange={(e) => setContent(e.target.value)}
              rows={4}
              className="w-full px-3 py-2 border border-gray-300 rounded-md focus:outline-none focus:ring-2 focus:ring-blue-500"
              placeholder="What's on your mind?"
            />
          </div>
        )}

        {postType === "image" && (
          <>
            <div className="mb-4">
              <label className="block text-sm font-medium text-gray-700 mb-2">
                Image
              </label>
              <input
                type="file"
                accept="image/*"
                onChange={handleImageUpload}
                className="w-full px-3 py-2 border border-gray-300 rounded-md focus:outline-none focus:ring-2 focus:ring-blue-500"
              />
            </div>
            <div className="mb-4">
              <label className="block text-sm font-medium text-gray-700 mb-2">
                Caption (optional)
              </label>
              <textarea
                value={caption}
                onChange={(e) => setCaption(e.target.value)}
                rows={2}
                className="w-full px-3 py-2 border border-gray-300 rounded-md focus:outline-none focus:ring-2 focus:ring-blue-500"
                placeholder="Add a caption..."
              />
            </div>
          </>
        )}

        {postType === "link" && (
          <>
            <div className="mb-4">
              <label className="block text-sm font-medium text-gray-700 mb-2">
                URL
              </label>
              <input
                type="url"
                value={url}
                onChange={(e) => handleUrlChange(e.target.value)}
                className="w-full px-3 py-2 border border-gray-300 rounded-md focus:outline-none focus:ring-2 focus:ring-blue-500"
                placeholder="https://example.com"
              />
            </div>
            <div className="mb-4">
              <label className="block text-sm font-medium text-gray-700 mb-2">
                Title/Description (optional)
              </label>
              <textarea
                value={content}
                onChange={(e) => setContent(e.target.value)}
                rows={2}
                className="w-full px-3 py-2 border border-gray-300 rounded-md focus:outline-none focus:ring-2 focus:ring-blue-500"
                placeholder="Add a title or description..."
              />
            </div>
          </>
        )}

        <button
          type="submit"
          className="w-full bg-blue-500 text-white py-2 px-4 rounded-md hover:bg-blue-600 focus:outline-none focus:ring-2 focus:ring-blue-500"
        >
          Post
        </button>
      </form>
    </div>
  );
};

