import {describe, it, expect, vi} from 'vitest';
import {patchStore, removeNonSerializable} from '../../src/middleware/patching';

describe('patching utilities', () => {
  describe('removeNonSerializable', () => {
    it('removes functions from objects', () => {
      const input = {
        count: 1,
        increment: () => {},
        nested: {
          value: 'test',
          callback: () => {},
        },
      };

      const result = removeNonSerializable(input);

      expect(result).toEqual({
        count: 1,
        nested: {
          value: 'test',
        },
      });
      expect(typeof result.increment).toBe('undefined');
      expect(typeof result.nested!.callback).toBe('undefined');
    });

    it('handles arrays correctly', () => {
      const input = {
        items: [
          {id: 1, name: 'Item 1', onClick: () => {}},
          {id: 2, name: 'Item 2', onClick: () => {}},
        ],
        processItems: () => {},
      };

      const result = removeNonSerializable(input);

      expect(result).toEqual({
        items: [
          {id: 1, name: 'Item 1'},
          {id: 2, name: 'Item 2'},
        ],
      });
    });

    it('handles null and undefined values', () => {
      const input = {
        nullValue: null,
        undefinedValue: undefined,
        validValue: 'test',
      };

      const result = removeNonSerializable(input);

      // With SuperJSON, the result includes metadata for special values like undefined
      // When deserialized, it should restore the original values
      expect(result).toHaveProperty('json');
      expect(result).toHaveProperty('meta');
      expect(result.json.nullValue).toBe(null);
      expect(result.json.validValue).toBe('test');
      
      // The undefined value is preserved in metadata and will be restored during deserialization
      expect(result.meta.values).toHaveProperty('undefinedValue');
    });

    it('returns non-object values as is', () => {
      expect(removeNonSerializable(42)).toBe(42);
      expect(removeNonSerializable('string')).toBe('string');
      expect(removeNonSerializable(null)).toBe(null);
      expect(removeNonSerializable(undefined)).toBe(undefined);
    });
  });

  describe('patchStore', () => {
    it('updates the store with document data', () => {
      const setState = vi.fn();
      // Add a mock getState function that returns an empty object
      const getState = vi.fn().mockReturnValue({});
      const api = {setState, getState};

      const docData = {count: 5, items: ['a', 'b']};
      patchStore(api, docData);

      expect(setState).toHaveBeenCalledTimes(1);

      // Verify setState was called with a function that properly merges state
      const setStateFn = setState.mock.calls[0][0];
      const result = setStateFn({otherProp: 'value'});
      expect(result).toEqual({
        otherProp: 'value',
        count: 5,
        items: ['a', 'b'],
      });
    });

    it('skips update when states are identical', () => {
      const setState = vi.fn();
      const initialState = {count: 5, items: ['a', 'b']};
      const getState = vi.fn().mockReturnValue(initialState);
      const api = {setState, getState};

      // Use the same data as current state
      const docData = {count: 5, items: ['a', 'b']};
      patchStore(api, docData);

      // setState should not be called when states are identical
      expect(setState).not.toHaveBeenCalled();
    });
  });
});
