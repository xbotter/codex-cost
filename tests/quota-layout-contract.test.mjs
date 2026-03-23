import test from "node:test";
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { resolve } from "node:path";

const stylesPath = resolve(import.meta.dirname, "..", "src", "styles.css");

test("heatmap-expanded quota row keeps full summary width", async () => {
  const source = await readFile(stylesPath, "utf8");
  const heatmapQuotaRowBlock = source.match(
    /\.hero-main\.is-heatmap-open \.quota-row\s*\{([\s\S]*?)\}/,
  )?.[1];

  assert.ok(heatmapQuotaRowBlock, "expected a dedicated heatmap-expanded quota-row block");
  assert.match(
    heatmapQuotaRowBlock,
    /width:\s*100%;/,
    "expected heatmap-expanded quota row to fill the summary column width",
  );
  assert.doesNotMatch(
    heatmapQuotaRowBlock,
    /max-width:\s*220px;/,
    "heatmap-expanded quota row should not clamp progress width to 220px",
  );
});
