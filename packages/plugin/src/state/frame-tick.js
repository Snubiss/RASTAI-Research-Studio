// One shared requestAnimationFrame loop (layout frontend-and-plugins.md 2.2 and 2.8): views of high-rate streams
// re-read their buffers once per frame instead of once per event. The loop runs only while someone listens, so an
// idle window schedules no frames.

/** @type {Set<() => void>} */
const frameListeners = new Set();
let frameRequested = false;

/**
 * Calls the listener once per animation frame until the returned function is called.
 * @param {() => void} listener
 * @returns {() => void} stops the calls
 */
export function onEveryFrame(listener) {
  frameListeners.add(listener);
  requestFrame();
  return () => {
    frameListeners.delete(listener);
  };
}

function requestFrame() {
  if (frameRequested || frameListeners.size === 0) {
    return;
  }
  frameRequested = true;
  requestAnimationFrame(runFrame);
}

function runFrame() {
  frameRequested = false;
  // A copy, so a listener that stops itself during the frame doesn't make the loop skip the next one.
  for (const listener of [...frameListeners]) {
    listener();
  }
  requestFrame();
}
