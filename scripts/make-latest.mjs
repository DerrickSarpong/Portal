// Generates the Tauri updater manifest (latest.json) from a finished release
// build. Upload it next to the NSIS setup exe in the GitHub release — the
// updater fetches it from `releases/latest/download/latest.json` to learn
// which version and artifact are current.
//
// Usage: node scripts/make-latest.mjs
//   PORTAL_REPO=owner/name  — override the GitHub repo slug (default: the
//   DerrickSarpong/Portal release stream configured in tauri.conf.json).
import { readFile, writeFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import path from "node:path";

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const conf = JSON.parse(
  await readFile(path.join(repoRoot, "src-tauri", "tauri.conf.json"), "utf8"),
);
const version = conf.version;
const slug = process.env.PORTAL_REPO ?? "DerrickSarpong/Portal";
const tag = `v${version}`;
const exeName = `Portal_${version}_x64-setup.exe`;

// The signature file sits next to the setup exe (bundle.createUpdaterArtifacts).
const bundleDir = path.join(repoRoot, "src-tauri", "target", "release", "bundle", "nsis");
const signature = (await readFile(path.join(bundleDir, `${exeName}.sig`), "utf8")).trim();

const latest = {
  version,
  pub_date: new Date().toISOString(),
  platforms: {
    "windows-x86_64": {
      signature,
      url: `https://github.com/${slug}/releases/download/${tag}/${exeName}`,
    },
  },
};

const out = path.join(bundleDir, "latest.json");
await writeFile(out, JSON.stringify(latest, null, 2) + "\n");
console.log(`Wrote ${out}`);
console.log(`Upload ${path.join(bundleDir, exeName)} and ${out} to the GitHub release ${tag}.`);
