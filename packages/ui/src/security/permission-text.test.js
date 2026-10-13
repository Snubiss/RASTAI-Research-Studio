// Plugin permissions in plain English (contracts 6.3): gpu, weights.read and conversations.read are the sensitive
// three, and anything unknown is shown raw and marked sensitive.

import { expect, test } from "vitest";

import { badgeText, egressDescription } from "./EgressBadge.jsx";
import { permissionText } from "./permission-text.js";

test("the sensitive three are marked, and only they among the known permissions", () => {
  const known = [
    "fs:project|ro",
    "secrets",
    "gpu",
    "weights.read",
    "conversations.read",
    "gateway.spend",
    "sandbox.spawn",
  ];
  const sensitive = known.filter((permission) => permissionText(permission).sensitive);
  expect(sensitive).toEqual(["gpu", "weights.read", "conversations.read"]);
});

test("list permissions name their items", () => {
  expect(permissionText("commands:[read, chat]").text).toBe("Run commands that need these permissions: read, chat");
  expect(permissionText("network:[huggingface.co]").text).toBe(
    "Connect to these internet hosts from its Python code: huggingface.co",
  );
});

test("an unknown or misspelled permission is shown as written and marked sensitive", () => {
  for (const permission of ["gpus", "network:huggingface.co", "filesystem:[/]", ""]) {
    expect(permissionText(permission)).toEqual({
      permission,
      text: `Unknown permission: ${permission}`,
      sensitive: true,
    });
  }
});

test("the egress badge text counts hosts once", () => {
  const egress = [
    { purpose: "model download", host: "huggingface.co", subjectId: "a" },
    { purpose: "dataset download", host: "huggingface.co", subjectId: "b" },
  ];
  expect(badgeText(egress)).toBe("Sending data to huggingface.co");
  expect(egressDescription(egress)).toBe(
    "Data is leaving this machine: huggingface.co (model download), huggingface.co (dataset download)",
  );
});
