export type PostType = "text" | "image" | "link";

export interface BasePost {
  id: string;
  type: PostType;
  authorId: string; // User ID instead of author name
  timestamp: number;
  contentRef: string; // Reference to keepsync content file
}

export interface TextPost extends BasePost {
  type: "text";
}

export interface ImagePost extends BasePost {
  type: "image";
}

export interface LinkPost extends BasePost {
  type: "link";
}

export type Post = TextPost | ImagePost | LinkPost;

// Content types stored in separate keepsync files
export interface TextContent {
  text: string;
}

export interface ImageContent {
  imageData: string; // base64 encoded image
  caption?: string;
}

export interface LinkContent {
  url: string;
  title?: string;
  description?: string;
}

export interface PostsState {
  posts: Post[];
  addPost: (post: Omit<Post, "id" | "timestamp">) => void;
  deletePost: (id: string) => void;
  getPosts: () => Post[];
}

