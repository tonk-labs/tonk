import { create } from "zustand";
import { sync, DocumentId } from "@tonk/keepsync";
import { Post, PostsState } from "../types/posts";
import { ContentStorage } from "../utils/contentStorage";

export const usePostsStore = create<PostsState>(
  sync(
    (set, get) => ({
      posts: [],

      addPost: (postData) => {
        const newPost: Post = {
          ...postData,
          id: crypto.randomUUID(),
          timestamp: Date.now(),
        };

        set((state) => ({
          posts: [newPost, ...state.posts].sort(
            (a, b) => b.timestamp - a.timestamp,
          ),
        }));
      },

      deletePost: async (id) => {
        const post = get().posts.find(p => p.id === id);
        if (post) {
          // Delete the content file
          await ContentStorage.deleteContent(post.contentRef);
        }
        
        set((state) => ({
          posts: state.posts.filter((post) => post.id !== id),
        }));
      },

      getPosts: () => {
        return get().posts.sort((a, b) => b.timestamp - a.timestamp);
      },
    }),
    {
      docId: "media-posts" as DocumentId,
      initTimeout: 30000,
      onInitError: (error) =>
        console.error("Posts sync initialization error:", error),
    },
  ),
);

