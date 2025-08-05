import { describe, it, expect, vi } from 'vitest';
import { patchStore } from '../../src/middleware/patching';

describe('patching utilities', () => {
  describe('patchStore', () => {
    it('updates the store with document data', () => {
      const setState = vi.fn();
      // Add a mock getState function that returns an empty object
      const getState = vi.fn().mockReturnValue({});
      const api = { setState, getState };

      const docData = { count: 5, items: ['a', 'b'] };
      patchStore(api, docData);

      expect(setState).toHaveBeenCalledTimes(1);

      // Verify setState was called with a function that properly merges state
      const setStateFn = setState.mock.calls[0][0];
      const result = setStateFn({ otherProp: 'value' });
      expect(result).toEqual({
        otherProp: 'value',
        count: 5,
        items: ['a', 'b'],
      });
    });

    it('skips update when states are identical', () => {
      const setState = vi.fn();
      const initialState = { count: 5, items: ['a', 'b'] };
      const getState = vi.fn().mockReturnValue(initialState);
      const api = { setState, getState };

      // Use the same data as current state
      const docData = { count: 5, items: ['a', 'b'] };
      patchStore(api, docData);

      // setState should not be called when states are identical
      expect(setState).not.toHaveBeenCalled();
    });
  });
});
