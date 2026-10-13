// High-rate streams: streamed chat text, live packets, metric series, log lines (layout frontend-and-plugins.md
// 2.2 and 2.3). The event hub pushes each item in O(1) and nothing renders per item; a view re-reads the buffer at
// most once per animation frame with useLiveBuffer. The buffer is a fixed ring, so memory stays flat however long
// a stream runs.

import { useEffect, useRef, useState } from "react";

import { onEveryFrame } from "./frame-tick.js";

/**
 * A ring of the newest items from a stream. Old items fall off the front. Items are numbered from 0 in the order
 * they were pushed; the item numbered `sequence` sits at items[sequence % capacity].
 * @typedef {object} LiveBuffer
 * @property {any[]} items
 * @property {number} capacity
 * @property {number} firstSequence   sequence number of the oldest item still held
 * @property {number} nextSequence    sequence number the next pushed item gets
 */

const DEFAULT_LIVE_BUFFER_CAPACITY = 16384;

/**
 * @param {number} [capacity]   how many of the newest items it keeps
 * @returns {LiveBuffer}
 */
export function createLiveBuffer(capacity = DEFAULT_LIVE_BUFFER_CAPACITY) {
  if (!Number.isInteger(capacity) || capacity < 1) {
    throw new RangeError(`A live buffer's capacity must be a positive whole number, not ${capacity}.`);
  }
  return { items: new Array(capacity), capacity, firstSequence: 0, nextSequence: 0 };
}

/**
 * @param {LiveBuffer} buffer
 * @param {any} item
 */
export function pushItem(buffer, item) {
  buffer.items[buffer.nextSequence % buffer.capacity] = item;
  buffer.nextSequence += 1;
  if (buffer.nextSequence - buffer.firstSequence > buffer.capacity) {
    buffer.firstSequence += 1;
  }
}

/**
 * @param {LiveBuffer} buffer
 * @param {number} sequence
 * @returns {any[]} the items from that sequence on (fewer if they fell off the front)
 */
export function itemsSince(buffer, sequence) {
  const items = [];
  for (let current = Math.max(sequence, buffer.firstSequence); current < buffer.nextSequence; current += 1) {
    items.push(buffer.items[current % buffer.capacity]);
  }
  return items;
}

/**
 * Re-renders the component at most once per animation frame, and only when items were pushed since the last one.
 * @param {LiveBuffer} buffer
 * @returns {number} the buffer's nextSequence
 */
export function useLiveBuffer(buffer) {
  const [renderedSequence, setRenderedSequence] = useState(buffer.nextSequence);
  // Compared here rather than left to React, which may still render once for a state set to the same value.
  const lastSequence = useRef(buffer.nextSequence);
  useEffect(() => {
    return onEveryFrame(() => {
      if (buffer.nextSequence !== lastSequence.current) {
        lastSequence.current = buffer.nextSequence;
        setRenderedSequence(buffer.nextSequence);
      }
    });
  }, [buffer]);
  return renderedSequence;
}
