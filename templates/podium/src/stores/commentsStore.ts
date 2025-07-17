import { create } from "zustand";
import { sync, DocumentId } from "@tonk/keepsync";
import { Comment, CommentsState } from "../types/users";

export const useCommentsStore = create<CommentsState>(
  sync(
    (set, get) => ({
      comments: [],

      addComment: (postId, authorId, content) => {
        const newComment: Comment = {
          id: crypto.randomUUID(),
          postId,
          authorId,
          content,
          timestamp: Date.now(),
        };

        set((state) => ({
          comments: [...state.comments, newComment].sort(
            (a, b) => a.timestamp - b.timestamp,
          ),
        }));
      },

      getCommentsForPost: (postId) => {
        return get()
          .comments.filter((comment) => comment.postId === postId)
          .sort((a, b) => a.timestamp - b.timestamp);
      },

      deleteComment: (commentId) => {
        set((state) => ({
          comments: state.comments.filter(
            (comment) => comment.id !== commentId,
          ),
        }));
      },
    }),
    {
      docId: "podium/comments" as DocumentId,
      initTimeout: 30000,
      onInitError: (error) =>
        console.error("Comments sync initialization error:", error),
    },
  ),
);
