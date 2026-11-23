import assert from "node:assert/strict";
import { parseNetUse, toDrivePath } from "./windows-drive.mjs";

const sample = `
Status       Local     Remote                      Network
-------------------------------------------------------------------------------
OK           X:        \\\\fileserver\\My Projects   Microsoft Windows Network
OK           Y:        \\\\server\\share             Microsoft Windows Network
`;

const maps = parseNetUse(sample);
assert.deepEqual(maps[0], ["\\\\fileserver\\My Projects", "X:"]);
assert.equal(
  toDrivePath("\\\\fileserver\\My Projects\\apps\\seda\\index.html", maps),
  "X:\\apps\\seda\\index.html",
);
assert.equal(toDrivePath("\\\\server\\share\\app\\index.html", maps), "Y:\\app\\index.html");
console.log("windows-drive ok");
