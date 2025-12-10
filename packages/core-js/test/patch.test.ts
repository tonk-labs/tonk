import assert from 'node:assert';
import { afterEach, beforeEach, describe, test } from 'node:test';
import { TonkCore } from '../dist/index.js';

describe('patchFile', () => {
  let tonk: TonkCore;

  beforeEach(async () => {
    tonk = await TonkCore.create();
  });

  afterEach(() => {
    if (tonk) {
      tonk.free();
    }
  });

  test('should patch a top-level field', async () => {
    await tonk.createFile('/position.json', { x: 100, y: 200 });

    const updated = await tonk.patchFile('/position.json', ['x'], 150);

    assert.strictEqual(updated, true);

    // Verify the patch was applied
    const doc = await tonk.readFile('/position.json');
    assert.strictEqual((doc.content as any).x, 150);
    assert.strictEqual((doc.content as any).y, 200); // y should be unchanged
  });

  test('should patch a nested field', async () => {
    await tonk.createFile('/config.json', {
      settings: {
        theme: 'light',
        fontSize: 14,
      },
    });

    const updated = await tonk.patchFile(
      '/config.json',
      ['settings', 'theme'],
      'dark'
    );

    assert.strictEqual(updated, true);

    const doc = await tonk.readFile('/config.json');
    assert.strictEqual((doc.content as any).settings.theme, 'dark');
    assert.strictEqual((doc.content as any).settings.fontSize, 14); // unchanged
  });

  test('should preserve other fields when patching', async () => {
    await tonk.createFile('/multi.json', { a: 1, b: 2, c: 3 });

    await tonk.patchFile('/multi.json', ['b'], 999);

    const doc = await tonk.readFile('/multi.json');
    assert.strictEqual((doc.content as any).a, 1);
    assert.strictEqual((doc.content as any).b, 999);
    assert.strictEqual((doc.content as any).c, 3);
  });

  test('should return false for non-existent file', async () => {
    const result = await tonk.patchFile('/nonexistent.json', ['x'], 100);
    assert.strictEqual(result, false);
  });

  test('should handle various value types', async () => {
    await tonk.createFile('/types.json', {
      num: 0,
      bool: false,
      str: '',
      obj: {},
      arr: [],
    });

    // Patch with number
    await tonk.patchFile('/types.json', ['num'], 42);
    // Patch with boolean
    await tonk.patchFile('/types.json', ['bool'], true);
    // Patch with null
    await tonk.patchFile('/types.json', ['str'], null);
    // Patch with object
    await tonk.patchFile('/types.json', ['obj'], { nested: 'value' });
    // Patch with array
    await tonk.patchFile('/types.json', ['arr'], [1, 2, 3]);

    const doc = await tonk.readFile('/types.json');
    const content = doc.content as any;

    assert.strictEqual(content.num, 42);
    assert.strictEqual(content.bool, true);
    assert.strictEqual(content.str, null);
    assert.deepStrictEqual(content.obj, { nested: 'value' });
    assert.deepStrictEqual(content.arr, [1, 2, 3]);
  });
});

describe('spliceText', () => {
  let tonk: TonkCore;

  beforeEach(async () => {
    tonk = await TonkCore.create();
  });

  afterEach(() => {
    if (tonk) {
      tonk.free();
    }
  });

  test('should insert text at position', async () => {
    await tonk.createFile('/text.json', { text: 'hello' });

    const updated = await tonk.spliceText(
      '/text.json',
      ['text'],
      5,
      0,
      ' world'
    );

    assert.strictEqual(updated, true);

    const doc = await tonk.readFile('/text.json');
    assert.strictEqual((doc.content as any).text, 'hello world');
  });

  test('should delete text at position', async () => {
    await tonk.createFile('/text.json', { text: 'hello world' });

    // Delete " world" (6 chars starting at position 5)
    const updated = await tonk.spliceText('/text.json', ['text'], 5, 6, '');

    assert.strictEqual(updated, true);

    const doc = await tonk.readFile('/text.json');
    assert.strictEqual((doc.content as any).text, 'hello');
  });

  test('should replace text', async () => {
    await tonk.createFile('/text.json', { text: 'hello world' });

    // Replace "world" with "universe"
    const updated = await tonk.spliceText(
      '/text.json',
      ['text'],
      6,
      5,
      'universe'
    );

    assert.strictEqual(updated, true);

    const doc = await tonk.readFile('/text.json');
    assert.strictEqual((doc.content as any).text, 'hello universe');
  });

  test('should create text field if missing', async () => {
    await tonk.createFile('/text.json', { other: 'value' });

    const updated = await tonk.spliceText(
      '/text.json',
      ['newtext'],
      0,
      0,
      'created'
    );

    assert.strictEqual(updated, true);

    const doc = await tonk.readFile('/text.json');
    assert.strictEqual((doc.content as any).newtext, 'created');
    assert.strictEqual((doc.content as any).other, 'value'); // unchanged
  });

  test('should return false for non-existent file', async () => {
    const result = await tonk.spliceText(
      '/nonexistent.json',
      ['text'],
      0,
      0,
      'text'
    );
    assert.strictEqual(result, false);
  });
});
