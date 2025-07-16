import React, { useState } from 'react';
import { useCommentsStore } from '../stores/commentsStore';
import { useSyncedUsersStore, useLocalAuthStore } from '../stores/userStore';

interface CommentSectionProps {
  postId: string;
}

export const CommentSection: React.FC<CommentSectionProps> = ({ postId }) => {
  const [newComment, setNewComment] = useState('');
  const { addComment, getCommentsForPost } = useCommentsStore();
  const { currentUser, isAuthenticated } = useLocalAuthStore();
  const { getUser } = useSyncedUsersStore();

  const postComments = getCommentsForPost(postId);

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    
    if (!isAuthenticated || !currentUser) {
      alert('Please log in to comment');
      return;
    }

    if (!newComment.trim()) {
      return;
    }

    addComment(postId, currentUser.id, newComment.trim());
    setNewComment('');
  };

  return (
    <div className="mt-4 border-t pt-4">
      {/* Comment form */}
      {isAuthenticated && currentUser && (
        <form onSubmit={handleSubmit} className="mb-4">
          <div className="flex space-x-2">
            <input
              type="text"
              value={newComment}
              onChange={(e) => setNewComment(e.target.value)}
              placeholder="Add a comment..."
              className="flex-1 px-3 py-2 border border-gray-300 rounded-md focus:outline-none focus:ring-2 focus:ring-blue-500"
            />
            <button
              type="submit"
              className="px-4 py-2 bg-blue-500 text-white rounded-md hover:bg-blue-600 focus:outline-none focus:ring-2 focus:ring-blue-500"
            >
              Comment
            </button>
          </div>
        </form>
      )}

      {/* Comments list */}
      <div className="space-y-3">
        {postComments.map((comment) => {
          const author = getUser(comment.authorId);
          return (
            <div key={comment.id} className="flex space-x-3">
              {author?.profilePicture && (
                <img 
                  src={author.profilePicture} 
                  alt={author.name}
                  className="w-8 h-8 rounded-full object-cover flex-shrink-0"
                />
              )}
              <div className="flex-1">
                <div className="bg-gray-100 rounded-lg px-3 py-2">
                  <div className="flex items-center space-x-2 mb-1">
                    <span className="font-semibold text-sm">
                      {author?.name || 'Unknown User'}
                    </span>
                    {author && !author.isOwner && (
                      <span className="text-xs text-gray-500">
                        ({author.relationToOwner})
                      </span>
                    )}
                    {author?.isOwner && (
                      <span className="text-xs bg-blue-100 text-blue-800 px-2 py-0.5 rounded">
                        Owner
                      </span>
                    )}
                  </div>
                  <p className="text-sm">{comment.content}</p>
                </div>
                <div className="text-xs text-gray-500 mt-1">
                  {new Date(comment.timestamp).toLocaleString()}
                </div>
              </div>
            </div>
          );
        })}
        
        {postComments.length === 0 && (
          <p className="text-gray-500 text-sm">No comments yet. Be the first to comment!</p>
        )}
      </div>
    </div>
  );
};