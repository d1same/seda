export type Piece = { text: string; kind: "same" | "add" | "del" };

const tokenize = (s: string) => s.match(/\s+|[^\s]+/g) ?? [];

/** Word-level diff (LCS) between two texts, keeping whitespace tokens. Deletions come before insertions. */
export function diffWords(a: string, b: string): Piece[] {
  const x = tokenize(a), y = tokenize(b);
  const n = x.length, m = y.length;
  const dp: number[][] = Array.from({ length: n + 1 }, () => new Array(m + 1).fill(0));
  for (let i = n - 1; i >= 0; i--)
    for (let j = m - 1; j >= 0; j--)
      dp[i][j] = x[i] === y[j] ? dp[i + 1][j + 1] + 1 : Math.max(dp[i + 1][j], dp[i][j + 1]);
  const out: Piece[] = [];
  const push = (text: string, kind: Piece["kind"]) => {
    const last = out[out.length - 1];
    if (last && last.kind === kind) last.text += text;
    else out.push({ text, kind });
  };
  let i = 0, j = 0;
  while (i < n || j < m) {
    if (i < n && j < m && x[i] === y[j]) { push(x[i], "same"); i++; j++; }
    else if (i < n && (j === m || dp[i + 1][j] >= dp[i][j + 1])) { push(x[i], "del"); i++; }
    else { push(y[j], "add"); j++; }
  }
  return out;
}

/** Number of separate places that changed (a replaced word counts once). */
export function changeCount(p: Piece[]): number {
  let groups = 0, inChange = false;
  for (const piece of p) {
    if (piece.kind === "same") {
      if (piece.text.trim()) inChange = false;
    } else if (!inChange) {
      groups++;
      inChange = true;
    }
  }
  return groups;
}
