// The live buffer's ring (layout frontend-and-plugins.md 2.3 and 7.5): wrap-around, and itemsSince across the wrap.

import { describe, expect, test } from "vitest";

import { createLiveBuffer, itemsSince, pushItem } from "./live-buffer.js";

/**
 * @param {number} capacity
 * @param {number} count   pushes the items 0, 1, ..., count - 1
 */
function bufferWith(capacity, count) {
  const buffer = createLiveBuffer(capacity);
  for (let item = 0; item < count; item += 1) {
    pushItem(buffer, item);
  }
  return buffer;
}

describe("createLiveBuffer", () => {
  test("starts empty with the default capacity", () => {
    const buffer = createLiveBuffer();
    expect(buffer.capacity).toBe(16384);
    expect(buffer.firstSequence).toBe(0);
    expect(buffer.nextSequence).toBe(0);
    expect(itemsSince(buffer, 0)).toEqual([]);
  });

  test("refuses a capacity that isn't a positive whole number", () => {
    expect(() => createLiveBuffer(0)).toThrow(RangeError);
    expect(() => createLiveBuffer(2.5)).toThrow(RangeError);
  });
});

describe("pushItem and itemsSince", () => {
  test("keep every item while the ring has room", () => {
    const buffer = bufferWith(4, 3);
    expect(itemsSince(buffer, 0)).toEqual([0, 1, 2]);
    expect(itemsSince(buffer, 2)).toEqual([2]);
    expect(itemsSince(buffer, 3)).toEqual([]);
  });

  test("drop the oldest items once the ring wraps", () => {
    const buffer = bufferWith(4, 10);
    expect(buffer.firstSequence).toBe(6);
    expect(buffer.nextSequence).toBe(10);
    expect(itemsSince(buffer, 0)).toEqual([6, 7, 8, 9]);
  });

  test("read across the wrap point in push order", () => {
    // Items 6 and 7 sit at the end of the ring's storage, 8 and 9 at its start.
    const buffer = bufferWith(4, 10);
    expect(itemsSince(buffer, 7)).toEqual([7, 8, 9]);
    expect(itemsSince(buffer, 9)).toEqual([9]);
  });

  test("return fewer items when the ones asked for fell off the front", () => {
    const buffer = bufferWith(3, 5);
    expect(itemsSince(buffer, 1)).toEqual([2, 3, 4]);
  });
});
