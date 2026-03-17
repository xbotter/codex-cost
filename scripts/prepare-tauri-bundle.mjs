import { spawn } from "node:child_process";
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import path from "node:path";

const cwd = process.cwd();
const bundleScriptPath = path.join(
  cwd,
  "src-tauri",
  "target",
  "release",
  "bundle",
  "dmg",
  "bundle_dmg.sh",
);

if (process.argv.includes("--watch")) {
  const deadline = Date.now() + 120_000;
  const timer = setInterval(() => {
    if (Date.now() > deadline) {
      clearInterval(timer);
      process.exit(0);
    }

    if (!existsSync(bundleScriptPath)) {
      return;
    }

    const content = readFileSync(bundleScriptPath, "utf8");
    if (!content.includes("SKIP_JENKINS=0")) {
      clearInterval(timer);
      process.exit(0);
    }

    writeFileSync(
      bundleScriptPath,
      content.replace("SKIP_JENKINS=0", "SKIP_JENKINS=1"),
      "utf8",
    );
    clearInterval(timer);
    process.exit(0);
  }, 200);
} else if (process.platform === "darwin") {
  const child = spawn(process.execPath, [new URL(import.meta.url).pathname, "--watch"], {
    cwd,
    detached: true,
    stdio: "ignore",
  });
  child.unref();
}
