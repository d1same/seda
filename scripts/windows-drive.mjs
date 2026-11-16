// Vite asks Windows for the real path of a mapped drive, then tries to turn
// "\\server\share" back into "Z:". Its parser stops at the first space, so
// "\\10.1.20.4\Ai Projects" becomes "\\10.1.20.4\Ai" and the project path
// loses the "Ai". Rewrite those paths ourselves, before Vite sees them.
import fs from "node:fs";
import { execSync } from "node:child_process";

export function parseNetUse(stdout) {
  const maps = [];
  for (const line of String(stdout).split(/\r?\n/)) {
    const m = line.match(/(\w:)\s+(\\\\[^\r\n]+?)\s{2,}\S/);
    if (!m) continue;
    maps.push([m[2], m[1]]);
  }
  maps.sort((a, b) => b[0].length - a[0].length);
  return maps;
}

export function toDrivePath(real, maps) {
  if (typeof real !== "string" || !real.startsWith("\\\\")) return real;
  const lower = real.toLowerCase();
  for (const [unc, drive] of maps) {
    const prefix = unc.toLowerCase();
    if (lower === prefix || lower.startsWith(prefix + "\\")) return drive + real.slice(unc.length);
  }
  return real;
}

export function installWindowsDrivePatch() {
  if (process.platform !== "win32") return;
  let maps = [];
  try {
    maps = parseNetUse(execSync("net use", { encoding: "utf8", windowsHide: true }));
  } catch {
    return;
  }
  if (maps.length === 0) return;
  const realpathSync = fs.realpathSync;
  const native = fs.realpathSync.native;
  const wrapped = (p, options) => toDrivePath(realpathSync(p, options), maps);
  wrapped.native = (p, options) => toDrivePath(native(p, options), maps);
  fs.realpathSync = wrapped;
  fs.realpathSync.native = wrapped.native;
  const prealpath = fs.promises.realpath;
  fs.promises.realpath = (p, options) => prealpath(p, options).then((real) => toDrivePath(real, maps));
}

installWindowsDrivePatch();
