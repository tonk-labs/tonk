import React, { useState } from "react";
import { usePostsStore } from "../stores/postsStore";
import { useLocalAuthStore } from "../stores/userStore";
import { PostType } from "../types/posts";
import { ContentStorage } from "../utils/contentStorage";

export const PostCreator: React.FC = () => {
  const [postType, setPostType] = useState<PostType>("text");
  const [content, setContent] = useState("");
  const [url, setUrl] = useState("");
  const [caption, setCaption] = useState("");
  const [imageFile, setImageFile] = useState<File | null>(null);

  const { addPost } = usePostsStore();
  const { currentUser, isAuthenticated } = useLocalAuthStore();

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

    if (!isAuthenticated || !currentUser) {
      alert("Please log in to create posts");
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
            authorId: currentUser.id,
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
            authorId: currentUser.id,
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
            authorId: currentUser.id,
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

  if (!isAuthenticated || !currentUser) {
    return (
      <div className="card">
        <p>Please log in to create posts.</p>
      </div>
    );
  }



  return (
    <div className="card">
      <h2>Create a Post</h2>

      <form onSubmit={handleSubmit}>
        {/* Post type selector */}
        <div className="form-group">
          <label className="form-label">Post Type</label>
          <div className="btn-group">
            <button
              type="button"
              onClick={() => setPostType("text")}
              className={`btn btn-secondary ${postType === "text" ? "active" : ""}`}
            >
              Text
            </button>
            <button
              type="button"
              onClick={() => setPostType("image")}
              className={`btn btn-secondary ${postType === "image" ? "active" : ""}`}
            >
              Image
            </button>
            <button
              type="button"
              onClick={() => setPostType("link")}
              className={`btn btn-secondary ${postType === "link" ? "active" : ""}`}
            >
              Link
            </button>
          </div>
        </div>

        {/* Content based on post type */}
        {postType === "text" && (
          <div className="form-group">
            <label className="form-label">Message</label>
            <textarea
              value={content}
              onChange={(e) => setContent(e.target.value)}
              rows={4}
              className="form-textarea"
              placeholder="What's on your mind?"
            />
          </div>
        )}

        {postType === "image" && (
          <>
            <div className="form-group">
              <label className="form-label">Image</label>
              <input
                type="file"
                accept="image/*"
                onChange={handleImageUpload}
                className="form-input"
              />
            </div>
            <div className="form-group">
              <label className="form-label">Caption (optional)</label>
              <textarea
                value={caption}
                onChange={(e) => setCaption(e.target.value)}
                rows={2}
                className="form-textarea"
                placeholder="Add a caption..."
              />
            </div>
          </>
        )}

        {postType === "link" && (
          <>
            <div className="form-group">
              <label className="form-label">URL</label>
              <input
                type="url"
                value={url}
                onChange={(e) => handleUrlChange(e.target.value)}
                className="form-input"
                placeholder="https://example.com"
              />
            </div>
            <div className="form-group">
              <label className="form-label">Title/Description (optional)</label>
              <textarea
                value={content}
                onChange={(e) => setContent(e.target.value)}
                rows={2}
                className="form-textarea"
                placeholder="Add a title or description..."
              />
            </div>
          </>
        )}

        <button
          type="submit"
          className="btn btn-primary"
          style={{ width: "100%" }}
        >
          Post
        </button>
      </form>
    </div>
  );
};
