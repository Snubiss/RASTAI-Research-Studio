// Blobs against the fake daemon (layout frontend-and-plugins.md 3.5): the URL, reading bytes, uploading with
// progress, the upload size cap, and failures as Diagnostics.

import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";

import { blobUrl, readBlobBytes, uploadBlob } from "./blobs.js";
import { connectToDaemon } from "./connection.js";
import { MAX_BROWSER_UPLOAD_BYTES } from "./constants.js";
import { UPLOADED_BLOB_HASH, createFakeDaemon, fakeXmlHttpRequestClass } from "./testing/fake-daemon.js";

/** @typedef {import("./testing/fake-daemon.js").FakeDaemon} FakeDaemon */
/** @typedef {import("./types.js").DaemonClient} DaemonClient */

const BLOB_HASH = "blake3:" + "cd".repeat(32);
const BLOB_BYTES = new Uint8Array([7, 8, 9]);

/** @type {FakeDaemon} */
let daemon;

beforeEach(() => {
  daemon = createFakeDaemon();
  daemon.blobs[BLOB_HASH] = BLOB_BYTES;
  vi.stubGlobal("fetch", daemon.fetch);
  vi.stubGlobal("XMLHttpRequest", fakeXmlHttpRequestClass(daemon));
});

afterEach(() => {
  vi.unstubAllGlobals();
});

/** @returns {Promise<DaemonClient>} */
async function connect() {
  const client = await connectToDaemon({ baseUrl: daemon.baseUrl, credential: { kind: "cookie", sessionKey: "k" } });
  if (!("execute" in client)) {
    throw new Error(`connectToDaemon failed: ${client.code}`);
  }
  return client;
}

test("blobUrl is the daemon's blob path for the hash", async () => {
  const client = await connect();
  expect(blobUrl(client, BLOB_HASH)).toBe(`${daemon.baseUrl}/api/blobs/blake3%3A${"cd".repeat(32)}`);
});

describe("readBlobBytes", () => {
  test("returns the blob's bytes", async () => {
    const client = await connect();
    expect(await readBlobBytes(client, BLOB_HASH)).toEqual(BLOB_BYTES);
  });

  test("a missing blob rejects with the daemon's Diagnostic as the cause", async () => {
    const client = await connect();
    const failure = await readBlobBytes(client, "blake3:" + "00".repeat(32)).catch((error) => error);
    expect(failure.cause.code).toBe("RRS-S1002");
  });

  test("a refused session rejects with RRS-U0003", async () => {
    const client = await connect();
    daemon.refusal = "RRS-C1002";
    const failure = await readBlobBytes(client, BLOB_HASH).catch((error) => error);
    expect(failure.cause.code).toBe("RRS-U0003");
  });

  test("an unreachable daemon rejects with RRS-U0001", async () => {
    const client = await connect();
    daemon.reachable = false;
    const failure = await readBlobBytes(client, BLOB_HASH).catch((error) => error);
    expect(failure.cause.code).toBe("RRS-U0001");
  });
});

describe("uploadBlob", () => {
  test("reports progress and returns the BlobRef", async () => {
    const client = await connect();
    /** @type {Array<[number, number]>} */
    const progress = [];
    const file = new Blob(["hello world"], { type: "text/plain" });
    const blobRef = await uploadBlob(client, file, (bytesDone, bytesTotal) => progress.push([bytesDone, bytesTotal]));
    expect(progress).toEqual([
      [5, 11],
      [11, 11],
    ]);
    expect("hash" in blobRef && [blobRef.hash, blobRef.sizeBytes, blobRef.mediaType]).toEqual([
      UPLOADED_BLOB_HASH,
      11n,
      "text/plain",
    ]);
  });

  test("a file above MAX_BROWSER_UPLOAD_BYTES is RRS-U0006 and is never sent", async () => {
    const client = await connect();
    const requestsBefore = daemon.requests.length;
    // Only the size is read before the cap refuses it, so a stand-in spares the test 256 MiB.
    const hugeFile = /** @type {Blob} */ ({ size: MAX_BROWSER_UPLOAD_BYTES + 1, type: "" });
    const result = await uploadBlob(client, hugeFile, () => {});
    expect("code" in result && result.code).toBe("RRS-U0006");
    expect(daemon.requests.length).toBe(requestsBefore);
    // A Blob has no name, and the client holds no English of its own for one: the catalog's placeholder stays.
    expect("message" in result && result.message).toBe("{file_name} is too large to upload from the browser.");
  });

  test("a File above the cap is named in the RRS-U0006 message", async () => {
    const client = await connect();
    const hugeFile = new File(["x"], "weights.safetensors");
    Object.defineProperty(hugeFile, "size", { value: MAX_BROWSER_UPLOAD_BYTES + 1 });
    const result = await uploadBlob(client, hugeFile, () => {});
    expect("message" in result && result.message).toBe("weights.safetensors is too large to upload from the browser.");
  });

  test("a success whose body isn't a BlobRef resolves to RRS-U0001 instead of throwing", async () => {
    const client = await connect();
    daemon.uploadAnswer = "<html>a proxy's page</html>";
    const result = await uploadBlob(client, new Blob(["x"]), () => {});
    expect("code" in result && result.code).toBe("RRS-U0001");
  });

  test("a refused session is RRS-U0003 and an unreachable daemon RRS-U0001", async () => {
    const client = await connect();
    daemon.refusal = "RRS-C1002";
    const refused = await uploadBlob(client, new Blob(["x"]), () => {});
    expect("code" in refused && refused.code).toBe("RRS-U0003");
    daemon.refusal = null;
    daemon.reachable = false;
    const unreachable = await uploadBlob(client, new Blob(["x"]), () => {});
    expect("code" in unreachable && unreachable.code).toBe("RRS-U0001");
  });
});
