export interface User {
  id: string; // UUID passkey
  name: string;
  relationToOwner: string;
  profilePicture?: string; // base64 encoded image
  isOwner: boolean;
  createdAt: number;
}

// This interface is no longer needed since we split the stores
// Keeping it for reference but it's not used anymore

export interface Comment {
  id: string;
  postId: string;
  authorId: string;
  content: string;
  timestamp: number;
}

export interface CommentsState {
  comments: Comment[];
  addComment: (postId: string, authorId: string, content: string) => void;
  getCommentsForPost: (postId: string) => Comment[];
  deleteComment: (commentId: string) => void;
}