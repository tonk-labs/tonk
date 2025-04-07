import {stringToUuidV4Sync} from '../../src/utils/deterministicUuid';
import * as Uuid from 'uuid';
import {jest, describe, test, expect} from '@jest/globals';
import bs58check from 'bs58check';
import bs58 from 'bs58';

describe('stringToUuidV4Sync', () => {
  test('generates valid UUID v4 format', () => {
    const uuid = stringToUuidV4Sync('yahoo-finance-macro');
    // UUID v4 format: 8-4-4-4-12 characters with hyphens
    expect(Uuid.validate(uuid)).toBe(true);
    console.log('Generated UUID:', uuid);

    // Convert UUID to binary form first, then encode
    const binaryUuid = Uuid.parse(uuid);
    const encoded = bs58check.encode(binaryUuid);
    console.log('Encoded:', encoded);

    const decoded = bs58check.decodeUnsafe(encoded);
    const stringified = Uuid.stringify(decoded!);
    console.log('Decoded back:', stringified);
    expect(Uuid.validate(stringified)).toBe(true);

    expect(uuid).toMatch(
      /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/,
    );
  });

  test('is deterministic - same input produces same UUID', () => {
    const input = 'test-deterministic';
    const uuid1 = stringToUuidV4Sync(input);
    const uuid2 = stringToUuidV4Sync(input);

    expect(Uuid.validate(uuid1)).toBe(true);
    console.log('Generated UUIDs:', {uuid1, uuid2});

    expect(uuid1).toBe(uuid2);
  });

  test('different inputs produce different UUIDs', () => {
    const uuid1 = stringToUuidV4Sync('input1');
    const uuid2 = stringToUuidV4Sync('input2');

    expect(Uuid.validate(uuid1)).toBe(true);
    expect(uuid1).not.toBe(uuid2);
  });

  test('handles empty string input', () => {
    const uuid = stringToUuidV4Sync('');
    // Should still generate a valid UUID
    expect(Uuid.validate(uuid)).toBe(true);
    expect(uuid).toMatch(
      /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/,
    );
  });

  test('handles special characters in input', () => {
    const uuid = stringToUuidV4Sync('!@#$%^&*()_+');
    expect(uuid).toMatch(
      /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/,
    );
  });

  test('version bits are correctly set to v4', () => {
    const uuid = stringToUuidV4Sync('test-version');
    // The 13th character should be '4' for version 4 UUID
    expect(uuid.charAt(14)).toBe('4');
    // The 17th character should be '8', '9', 'a', or 'b' for variant 1
    expect(uuid.charAt(19)).toMatch(/[89ab]/);
  });
});
