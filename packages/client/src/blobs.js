// Blobs: a URL for <img> and download links, reading a blob's bytes, and uploading a file with progress (layout
// frontend-and-plugins.md 2.1, 3.2; contracts 3.3). Blobs are immutable, so the browser caches each one by hash.

import { BLOBS_PATH, CODE_UPLOAD_TOO_LARGE, MAX_BROWSER_UPLOAD_BYTES } from "./constants.js";
import { daemonUrl, requestHeaders } from "./connection.js";
import {
  callFailureDiagnostic,
  clientDiagnostic,
  diagnosticFromRefusalJson,
  failureDiagnostic,
} from "./diagnostics.js";

/** @typedef {import("@rastai/proto").BlobRef} BlobRef */
/** @typedef {import("@rastai/proto").Diagnostic} Diagnostic */
/** @typedef {import("./types.js").DaemonClient} DaemonClient */

// Protobuf-es messages are plain objects; this field names the message, as `create(BlobRefSchema)` would.
const BLOB_REF_TYPE_NAME = "rastai.common.v1.BlobRef";
const READ_METHOD = "GET";
const UPLOAD_METHOD = "PUT";
// The status a refusal carries when XMLHttpRequest reports a network error instead of a response.
const NO_RESPONSE_STATUS = 0;
const HTTP_OK_MIN = 200;
const HTTP_OK_MAX = 299;

/**
 * A URL for <img src> or a download link. Cookie credential only: the browser sends the cookie on a GET with no
 * Origin and no extra header (contracts 3.2); a bearer client reads bytes with readBlobBytes instead.
 * @param {DaemonClient} client
 * @param {string} blobHash   blake3:<64 hex>
 * @returns {string}
 */
export function blobUrl(client, blobHash) {
  return daemonUrl(client.baseUrl, `${BLOBS_PATH}/${encodeURIComponent(blobHash)}`);
}

/**
 * Reads a whole blob. A GET, so a cookie credential sends no session key; a bearer credential sends its token.
 * Rejects with an Error whose `cause` is the Diagnostic when the blob can't be read.
 * @param {DaemonClient} client
 * @param {string} blobHash
 * @returns {Promise<Uint8Array>}
 */
export async function readBlobBytes(client, blobHash) {
  let response;
  try {
    response = await fetch(blobUrl(client, blobHash), { headers: requestHeaders(client.credential, READ_METHOD) });
  } catch (error) {
    throw failure(callFailureDiagnostic(error, client.baseUrl));
  }
  if (!response.ok) {
    throw failure(await refusalOf(client, response.status, () => response.text()));
  }
  return new Uint8Array(await response.arrayBuffer());
}

/**
 * Uploads a file to the blob store. XMLHttpRequest instead of fetch, because only it reports upload progress.
 * @param {DaemonClient} client
 * @param {Blob} file
 * @param {(bytesDone: number, bytesTotal: number) => void} onProgress
 * @returns {Promise<BlobRef | Diagnostic>}   RRS-U0006 above MAX_BROWSER_UPLOAD_BYTES
 */
export async function uploadBlob(client, file, onProgress) {
  if (file.size > MAX_BROWSER_UPLOAD_BYTES) {
    const fileName = file instanceof File ? file.name : "This file";
    return clientDiagnostic(CODE_UPLOAD_TOO_LARGE, { file_name: fileName });
  }
  const reply = await sendUpload(client, file, onProgress);
  if (reply.status < HTTP_OK_MIN || reply.status > HTTP_OK_MAX) {
    return refusalOf(client, reply.status, async () => reply.body);
  }
  return blobRefFromJson(JSON.parse(reply.body));
}

/**
 * One PUT with progress. Resolves with the status and body; status 0 when no response came.
 * @param {DaemonClient} client
 * @param {Blob} file
 * @param {(bytesDone: number, bytesTotal: number) => void} onProgress
 * @returns {Promise<{ status: number, body: string }>}
 */
function sendUpload(client, file, onProgress) {
  return new Promise((resolve) => {
    const request = new XMLHttpRequest();
    request.open(UPLOAD_METHOD, daemonUrl(client.baseUrl, BLOBS_PATH));
    for (const [name, value] of Object.entries(requestHeaders(client.credential, UPLOAD_METHOD))) {
      request.setRequestHeader(name, value);
    }
    request.upload.onprogress = (progress) => onProgress(progress.loaded, progress.total);
    request.onload = () => resolve({ status: request.status, body: request.responseText });
    request.onerror = () => resolve({ status: NO_RESPONSE_STATUS, body: "" });
    request.send(file);
  });
}

/**
 * The Diagnostic for a blob request the daemon didn't answer with success. Its refusals on these plain HTTP paths
 * are the Diagnostic as JSON (contracts 3.2); an answer without one means something else answered, or nothing did.
 * @param {DaemonClient} client
 * @param {number} status
 * @param {() => Promise<string>} readBody
 * @returns {Promise<Diagnostic>}
 */
async function refusalOf(client, status, readBody) {
  if (status === NO_RESPONSE_STATUS) {
    return failureDiagnostic(null, client.baseUrl);
  }
  return failureDiagnostic(await readRefusal(readBody), client.baseUrl);
}

/**
 * @param {() => Promise<string>} readBody
 * @returns {Promise<Diagnostic | null>}
 */
async function readRefusal(readBody) {
  try {
    return diagnosticFromRefusalJson(JSON.parse(await readBody()));
  } catch {
    return null;
  }
}

/**
 * PUT /api/blobs answers the BlobRef as protobuf JSON with proto field names; size_bytes is a 64-bit integer, so
 * it comes as a string (contracts 4).
 * @param {any} json
 * @returns {BlobRef}
 */
function blobRefFromJson(json) {
  return {
    $typeName: BLOB_REF_TYPE_NAME,
    hash: String(json.hash ?? ""),
    sizeBytes: BigInt(json.size_bytes ?? 0),
    mediaType: String(json.media_type ?? ""),
  };
}

/**
 * @param {Diagnostic} diagnostic
 * @returns {Error}
 */
function failure(diagnostic) {
  return new Error(diagnostic.message, { cause: diagnostic });
}
