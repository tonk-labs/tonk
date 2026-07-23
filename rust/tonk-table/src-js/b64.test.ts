import { test } from "node:test";
import assert from "node:assert/strict";
import { base64ToBytes, bytesToBase64 } from "./b64";

test("bytes round-trip through base64", () => {
  const bytes = new Uint8Array([0, 1, 2, 250, 251, 255]);
  const back = base64ToBytes(bytesToBase64(bytes));
  assert.deepEqual(back, bytes);
});

test("a buffer larger than the encoding chunk round-trips", () => {
  // 0x8000 is the chunk size; cross several boundaries with a pattern
  // that isn't chunk-aligned.
  const bytes = new Uint8Array(0x8000 * 3 + 17);
  for (let i = 0; i < bytes.length; i++) bytes[i] = (i * 31) & 0xff;
  const back = base64ToBytes(bytesToBase64(bytes));
  assert.ok(back);
  assert.equal(back.length, bytes.length);
  assert.deepEqual(back, bytes);
});

test("empty bytes encode to an empty string and back", () => {
  const encoded = bytesToBase64(new Uint8Array(0));
  assert.equal(encoded, "");
  const back = base64ToBytes(encoded);
  assert.ok(back);
  assert.equal(back.length, 0);
});

test("whitespace inside the base64 is tolerated", () => {
  const encoded = bytesToBase64(new Uint8Array([1, 2, 3, 4, 5, 6]));
  const wrapped = `${encoded.slice(0, 4)}\r\n  ${encoded.slice(4)}`;
  assert.deepEqual(base64ToBytes(wrapped), new Uint8Array([1, 2, 3, 4, 5, 6]));
});

test("non-base64 input decodes to null, not garbage", () => {
  assert.equal(base64ToBytes("a,b\n1,2"), null);
  assert.equal(base64ToBytes("not base64!!!"), null);
  // Valid alphabet but impossible length (4n+1).
  assert.equal(base64ToBytes("AAAAA"), null);
});
