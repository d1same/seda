import assert from "node:assert/strict";
import { parseNetUse, toDrivePath } from "./windows-drive.mjs";

const sample = `
Status       Local     Remote                    Network
-------------------------------------------------------------------------------
OK           Z:        \\\\10.1.20.4\\Ai Projects   Microsoft Windows Network
OK           Y:        \\\\server\\share            Microsoft Windows Network
`;

const maps = parseNetUse(sample);
assert.deepEqual(maps[0], ["\\\\10.1.20.4\\Ai Projects", "Z:"]);
assert.equal(
  toDrivePath("\\\\10.1.20.4\\Ai Projects\\Projects\\Seda\\seda\\index.html", maps),
  "Z:\\Projects\\Seda\\seda\\index.html",
);
assert.equal(toDrivePath("\\\\server\\share\\app\\index.html", maps), "Y:\\app\\index.html");
console.log("windows-drive ok");
