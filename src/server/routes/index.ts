import { Router } from 'express';
import { DatabaseError } from '../../db/schema';

const router = Router();

// Generic error handler for routes
export const handleRouteError = (error: unknown) => {
  if (error instanceof DatabaseError) {
    return {
      status: 400,
      message: error.message
    };
  }
  
  return {
    status: 500,
    message: 'Internal server error'
  };
};

// Example route template
router.get('/health', (_req, res) => {
  res.json({ status: 'ok' });
});

export default router; 
