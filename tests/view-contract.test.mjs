import test from "node:test";
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { resolve } from "node:path";

const htmlPath = resolve(import.meta.dirname, "..", "index.html");
const mainTsPath = resolve(import.meta.dirname, "..", "src", "main.ts");

test("single-window view shell exposes the required root ids", async () => {
  const html = await readFile(htmlPath, "utf8");

  assert.match(html, /id="dashboard-view"/);
  assert.match(html, /id="settings-view"/);
  assert.match(html, /id="settings-back-button"/);
  assert.match(html, /id="settings-form"/);
  assert.match(html, /id="provider-list"/);
  assert.match(html, /id="provider-detail"/);
  assert.match(html, /id="form-status"/);
  assert.match(html, /id="save-button"/);
});

test("settings back button uses the shared SVG icon system", async () => {
  const [html, source] = await Promise.all([
    readFile(htmlPath, "utf8"),
    readFile(mainTsPath, "utf8"),
  ]);

  assert.doesNotMatch(html, /<span aria-hidden="true">←<\/span>/);
  assert.match(source, /arrow-left\.svg\?raw/);
  assert.match(source, /settingsBackButtonEl\.innerHTML = `\$\{backIconMarkup\(\)\}<span>Back<\/span>`;/);
});
