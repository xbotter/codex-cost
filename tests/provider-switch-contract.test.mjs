import test from "node:test";
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { resolve } from "node:path";

const mainTsPath = resolve(import.meta.dirname, "..", "src", "main.ts");

test("provider switching does not rely on redundant snapshot roundtrips", async () => {
  const source = await readFile(mainTsPath, "utf8");
  const getSnapshotCalls = source.match(/invoke<AppSnapshot>\("get_snapshot"\)/g) ?? [];

  assert.equal(
    getSnapshotCalls.length,
    1,
    `expected only the initial app bootstrap to call get_snapshot, found ${getSnapshotCalls.length}`,
  );
});
