import { describe, it, expect } from 'vitest';
import {
  serializeForSync,
  deserializeFromSync,
  areSerializedEqual,
} from '../../src/utils/serialization';

describe('Enhanced Serialization with SuperJSON', () => {
  describe('serializeForSync', () => {
    it('preserves Date objects', () => {
      const testDate = new Date('2024-01-15T10:30:00Z');
      const input = {
        createdAt: testDate,
        updatedAt: new Date('2024-01-16T15:45:00Z'),
        nested: {
          timestamp: testDate,
        },
      };

      const serialized = serializeForSync(input);
      const deserialized = deserializeFromSync(serialized) as any;

      expect(deserialized.createdAt).toBeInstanceOf(Date);
      expect(deserialized.createdAt.getTime()).toBe(testDate.getTime());
      expect(deserialized.nested.timestamp).toBeInstanceOf(Date);
      expect(deserialized.nested.timestamp.getTime()).toBe(testDate.getTime());
    });

    it('preserves Map objects', () => {
      const testMap = new Map();
      testMap.set('string-key', 'string-value');
      testMap.set('date-value', new Date('2024-01-01'));

      const input = {
        userPreferences: testMap,
        nested: {
          cache: new Map([['key1', { data: 'value1' }]]),
        },
      };

      const serialized = serializeForSync(input);
      const deserialized = deserializeFromSync(serialized) as any;

      expect(deserialized.userPreferences).toBeInstanceOf(Map);
      expect(deserialized.userPreferences.get('string-key')).toBe('string-value');
      expect(deserialized.userPreferences.get('date-value')).toBeInstanceOf(Date);
      expect(deserialized.nested.cache).toBeInstanceOf(Map);
      expect(deserialized.nested.cache.get('key1')).toEqual({ data: 'value1' });
    });

    it('preserves Set objects', () => {
      const testSet = new Set([
        'string-value',
        42,
        true,
        new Date('2024-01-01'),
        { object: 'value' },
      ]);

      const input = {
        tags: testSet,
        nested: {
          uniqueIds: new Set([1, 2, 3]),
        },
      };

      const serialized = serializeForSync(input);
      const deserialized = deserializeFromSync(serialized) as any;

      expect(deserialized.tags).toBeInstanceOf(Set);
      expect(deserialized.tags.has('string-value')).toBe(true);
      expect(deserialized.tags.has(42)).toBe(true);
      expect(Array.from(deserialized.tags).find((item: any) => item instanceof Date)).toBeInstanceOf(Date);
      expect(deserialized.nested.uniqueIds).toBeInstanceOf(Set);
      expect(deserialized.nested.uniqueIds.has(1)).toBe(true);
    });

    it('preserves RegExp objects', () => {
      const input = {
        emailPattern: /^[^\s@]+@[^\s@]+\.[^\s@]+$/,
        phonePattern: new RegExp('^\\+?[1-9]\\d{1,14}$', 'i'),
        nested: {
          validation: /^\d{4}-\d{2}-\d{2}$/,
        },
      };

      const serialized = serializeForSync(input);
      const deserialized = deserializeFromSync(serialized) as any;

      expect(deserialized.emailPattern).toBeInstanceOf(RegExp);
      expect(deserialized.emailPattern.source).toBe('^[^\\s@]+@[^\\s@]+\\.[^\\s@]+$');
      expect(deserialized.phonePattern).toBeInstanceOf(RegExp);
      expect(deserialized.phonePattern.flags).toBe('i');
      expect(deserialized.nested.validation).toBeInstanceOf(RegExp);
    });

    it('preserves BigInt values', () => {
      const input = {
        largeNumber: BigInt('9007199254740991'),
        calculations: {
          result: BigInt('123456789012345678901234567890'),
        },
        array: [BigInt('1'), BigInt('2'), BigInt('3')],
      };

      const serialized = serializeForSync(input);
      const deserialized = deserializeFromSync(serialized) as any;

      expect(typeof deserialized.largeNumber).toBe('bigint');
      expect(deserialized.largeNumber).toBe(BigInt('9007199254740991'));
      expect(typeof deserialized.calculations.result).toBe('bigint');
      expect(deserialized.array[0]).toBe(BigInt('1'));
    });

    it('removes functions while preserving other types', () => {
      const input = {
        name: 'test',
        createdAt: new Date('2024-01-01'),
        tags: new Set(['tag1', 'tag2']),
        metadata: new Map([['key', 'value']]),
        onClick: () => console.log('clicked'), // Should be removed
        nested: {
          data: 'value',
          callback: function() { return 'test'; }, // Should be removed
          pattern: /test/g,
        },
        items: [
          { id: 1, name: 'item1', handler: () => {} }, // handler should be removed
          { id: 2, name: 'item2', date: new Date('2024-01-02') },
        ],
      };

      const serialized = serializeForSync(input);
      const deserialized = deserializeFromSync(serialized) as any;

      // Functions should be removed
      expect(deserialized.onClick).toBeUndefined();
      expect(deserialized.nested.callback).toBeUndefined();
      expect(deserialized.items[0].handler).toBeUndefined();

      // Other types should be preserved
      expect(deserialized.name).toBe('test');
      expect(deserialized.createdAt).toBeInstanceOf(Date);
      expect(deserialized.tags).toBeInstanceOf(Set);
      expect(deserialized.metadata).toBeInstanceOf(Map);
      expect(deserialized.nested.pattern).toBeInstanceOf(RegExp);
      expect(deserialized.items[1].date).toBeInstanceOf(Date);
    });

    it('handles functions in Maps and Sets', () => {
      const mapWithFunctions = new Map();
      mapWithFunctions.set('data', 'value');
      mapWithFunctions.set('callback', () => 'test'); // Should be removed
      mapWithFunctions.set('date', new Date('2024-01-01'));

      const setWithFunctions = new Set([
        'string',
        () => 'function', // Should be removed
        new Date('2024-01-01'),
        42,
      ]);

      const input = {
        mapData: mapWithFunctions,
        setData: setWithFunctions,
      };

      const serialized = serializeForSync(input);
      const deserialized = deserializeFromSync(serialized) as any;

      // Map should preserve non-function values
      expect(deserialized.mapData).toBeInstanceOf(Map);
      expect(deserialized.mapData.get('data')).toBe('value');
      expect(deserialized.mapData.get('date')).toBeInstanceOf(Date);
      expect(deserialized.mapData.has('callback')).toBe(false);

      // Set should preserve non-function values
      expect(deserialized.setData).toBeInstanceOf(Set);
      expect(deserialized.setData.has('string')).toBe(true);
      expect(deserialized.setData.has(42)).toBe(true);
      expect(Array.from(deserialized.setData).find((item: any) => item instanceof Date)).toBeInstanceOf(Date);
      expect(Array.from(deserialized.setData).find((item: any) => typeof item === 'function')).toBeUndefined();
    });

    it('handles complex nested structures', () => {
      const complexInput = {
        user: {
          id: 'user-123',
          profile: {
            name: 'John Doe',
            createdAt: new Date('2024-01-01'),
            preferences: new Map([
              ['theme', 'dark'],
              ['lastLogin', new Date('2024-01-15')],
            ] as [string, any][]),
            tags: new Set(['admin', 'premium']),
            emailPattern: /^[^\s@]+@[^\s@]+\.[^\s@]+$/,
          },
          actions: {
            login: () => {}, // Should be removed
            logout: () => {}, // Should be removed
          },
        },
        data: {
          items: [
            {
              id: BigInt('1'),
              timestamp: new Date('2024-01-01'),
              metadata: new Map([['type', 'document']]),
              process: () => {}, // Should be removed
            },
            {
              id: BigInt('2'),
              timestamp: new Date('2024-01-02'),
              tags: new Set(['important']),
            },
          ],
          counters: new Map([
            ['views', BigInt('1000')],
            ['likes', BigInt('50')],
          ]),
        },
      };

      const serialized = serializeForSync(complexInput);
      const deserialized = deserializeFromSync(serialized) as any;

      // Verify structure is preserved
      expect(deserialized.user.profile.createdAt).toBeInstanceOf(Date);
      expect(deserialized.user.profile.preferences).toBeInstanceOf(Map);
      expect(deserialized.user.profile.tags).toBeInstanceOf(Set);
      expect(deserialized.user.profile.emailPattern).toBeInstanceOf(RegExp);

      // Verify functions are removed
      expect(deserialized.user.actions.login).toBeUndefined();
      expect(deserialized.user.actions.logout).toBeUndefined();
      expect(deserialized.data.items[0].process).toBeUndefined();

      // Verify complex types in arrays
      expect(deserialized.data.items[0].id).toBe(BigInt('1'));
      expect(deserialized.data.items[0].timestamp).toBeInstanceOf(Date);
      expect(deserialized.data.items[0].metadata).toBeInstanceOf(Map);
      expect(deserialized.data.items[1].tags).toBeInstanceOf(Set);

      // Verify Map with BigInt values
      expect(deserialized.data.counters).toBeInstanceOf(Map);
      expect(deserialized.data.counters.get('views')).toBe(BigInt('1000'));
    });

    it('handles primitive values correctly', () => {
      expect(serializeForSync(42)).toBe(42);
      expect(serializeForSync('string')).toBe('string');
      expect(serializeForSync(true)).toBe(true);
      expect(serializeForSync(null)).toBe(null);
      expect(serializeForSync(undefined)).toBe(undefined);
    });

    it('handles arrays with mixed types', () => {
      const input = [
        'string',
        42,
        new Date('2024-01-01'),
        new Map([['key', 'value']]),
        new Set([1, 2, 3]),
        /pattern/g,
        BigInt('123'),
        () => {}, // Should be removed
        {
          nested: 'object',
          callback: () => {}, // Should be removed
          date: new Date('2024-01-02'),
        },
      ];

      const serialized = serializeForSync(input);
      const deserialized = deserializeFromSync(serialized) as any;

      expect(Array.isArray(deserialized)).toBe(true);
      expect(deserialized[0]).toBe('string');
      expect(deserialized[1]).toBe(42);
      expect(deserialized[2]).toBeInstanceOf(Date);
      expect(deserialized[3]).toBeInstanceOf(Map);
      expect(deserialized[4]).toBeInstanceOf(Set);
      expect(deserialized[5]).toBeInstanceOf(RegExp);
      expect(deserialized[6]).toBe(BigInt('123'));
      
      // Function should be filtered out, so array should be shorter
      expect(deserialized.length).toBe(8); // 9 original items - 1 function
      
      // Check nested object
      const nestedObj = deserialized[7];
      expect(nestedObj.nested).toBe('object');
      expect(nestedObj.callback).toBeUndefined();
      expect(nestedObj.date).toBeInstanceOf(Date);
    });
  });

  describe('areSerializedEqual', () => {
    it('correctly identifies equal objects with complex types', () => {
      const date = new Date('2024-01-01');
      const map = new Map([['key', 'value']]);
      const set = new Set([1, 2, 3]);

      const obj1 = {
        date,
        map,
        set,
        regex: /test/g,
        bigint: BigInt('123'),
        func: () => {}, // Should be ignored in comparison
      };

      const obj2 = {
        date: new Date('2024-01-01'),
        map: new Map([['key', 'value']]),
        set: new Set([1, 2, 3]),
        regex: /test/g,
        bigint: BigInt('123'),
        func: () => {}, // Should be ignored in comparison
      };

      expect(areSerializedEqual(obj1, obj2)).toBe(true);
    });

    it('correctly identifies different objects', () => {
      const obj1 = {
        date: new Date('2024-01-01'),
        map: new Map([['key', 'value1']]),
      };

      const obj2 = {
        date: new Date('2024-01-01'),
        map: new Map([['key', 'value2']]),
      };

      expect(areSerializedEqual(obj1, obj2)).toBe(false);
    });

    it('ignores function differences', () => {
      const obj1 = {
        data: 'same',
        func1: () => 'different',
      };

      const obj2 = {
        data: 'same',
        func1: () => 'also different',
      };

      expect(areSerializedEqual(obj1, obj2)).toBe(true);
    });
  });

  describe('Error handling and edge cases', () => {
    it('handles circular references gracefully', () => {
      const obj: any = { name: 'test' };
      obj.self = obj; // Create circular reference

      // Should not throw an error
      expect(() => serializeForSync(obj)).not.toThrow();
    });

    it('handles empty objects and arrays', () => {
      expect(deserializeFromSync(serializeForSync({}))).toEqual({});
      expect(deserializeFromSync(serializeForSync([]))).toEqual([]);
    });

    it('handles null and undefined values', () => {
      const input = {
        nullValue: null,
        undefinedValue: undefined,
        nested: {
          alsoNull: null,
        },
      };

      const serialized = serializeForSync(input);
      const deserialized = deserializeFromSync(serialized) as any;

      expect(deserialized.nullValue).toBe(null);
      expect(deserialized.undefinedValue).toBe(undefined);
      expect(deserialized.nested.alsoNull).toBe(null);
    });

    it('handles legacy data without SuperJSON metadata', () => {
      const legacyData = {
        name: 'test',
        count: 42,
        items: ['a', 'b', 'c'],
      };

      // This simulates data that was serialized with the old method
      const deserialized = deserializeFromSync(legacyData);
      expect(deserialized).toEqual(legacyData);
    });

    it('handles malformed SuperJSON data gracefully', () => {
      const malformedData = {
        json: { name: 'test' },
        meta: 'invalid-meta', // Invalid meta format
      };

      // Should not throw and should return the data as-is
      expect(() => deserializeFromSync(malformedData)).not.toThrow();
    });
  });

  describe('Performance and large objects', () => {
    it('handles large objects efficiently', () => {
      const largeObject = {
        users: Array.from({ length: 100 }, (_, i) => ({
          id: i,
          name: `User ${i}`,
          createdAt: new Date(`2024-01-${(i % 28) + 1}`),
          preferences: new Map([
            ['theme', i % 2 === 0 ? 'dark' : 'light'],
          ] as [string, any][]),
          tags: new Set([`tag${i % 10}`, `category${i % 5}`]),
          process: () => {}, // Should be removed
        })),
        metadata: new Map(
          Array.from({ length: 50 }, (_, i) => [`key${i}`, `value${i}`])
        ),
      };

      const start = performance.now();
      const serialized = serializeForSync(largeObject);
      const deserialized = deserializeFromSync(serialized) as any;
      const end = performance.now();

      // Should complete in reasonable time (less than 1 second)
      expect(end - start).toBeLessThan(1000);

      // Verify structure is preserved
      expect(deserialized.users).toHaveLength(100);
      expect(deserialized.users[0].createdAt).toBeInstanceOf(Date);
      expect(deserialized.users[0].preferences).toBeInstanceOf(Map);
      expect(deserialized.users[0].tags).toBeInstanceOf(Set);
      expect(deserialized.users[0].process).toBeUndefined();
      expect(deserialized.metadata).toBeInstanceOf(Map);
    });
  });
});