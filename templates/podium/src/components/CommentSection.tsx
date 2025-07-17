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
    <div className="comments-section">
      {/* Comment form */}
      {isAuthenticated && currentUser && (
        <form onSubmit={handleSubmit} className="comment-form">
          <input
            type="text"
            value={newComment}
            onChange={(e) => setNewComment(e.target.value)}
            placeholder="Add a comment..."
            className="form-input comment-input"
          />
          <button
            type="submit"
            className="btn btn-primary"
          >
            Comment
          </button>
        </form>
      )}

      {/* Comments list */}
      <div className="comments-list">
        {postComments.map((comment) => {
          const author = getUser(comment.authorId);
          return (
            <div key={comment.id} className="comment-item">
              {author?.profilePicture && (
                <img 
                  src={author.profilePicture} 
                  alt={author.name}
                  className="comment-avatar"
                />
              )}
              <div className="comment-content">
                <div className="comment-bubble">
                  <div className="comment-author">
                    <span className="comment-author-name">
                      {author?.name || 'Unknown User'}
                    </span>
                    {author && !author.isOwner && (
                      <span className="comment-relation">
                        ({author.relationToOwner})
                      </span>
                    )}
                    {author?.isOwner && (
                      <span className="owner-badge">
                        Owner
                      </span>
                    )}
                  </div>
                  <p className="comment-text">{comment.content}</p>
                </div>
                <div className="comment-timestamp">
                  {new Date(comment.timestamp).toLocaleString()}
                </div>
              </div>
            </div>
          );
        })}
        
        {postComments.length === 0 && (
          <p style={{ opacity: 0.6, fontSize: '0.85rem' }}>No comments yet. Be the first to comment!</p>
        )}
      </div>
    </div>
  );
};