/** unified diff 渲染：统一视图 + 并排视图（D5）。 */

function classify(line: string): string {
  if (line.startsWith("+++") || line.startsWith("---")) return "dl-file";
  if (line.startsWith("@@")) return "dl-hunk";
  if (line.startsWith("+")) return "dl-add";
  if (line.startsWith("-")) return "dl-del";
  if (line.startsWith("diff ") || line.startsWith("index ")) return "dl-meta";
  return "dl";
}

function isDel(line: string) {
  return line.startsWith("-") && !line.startsWith("---");
}
function isAdd(line: string) {
  return line.startsWith("+") && !line.startsWith("+++");
}

export type SplitRow = {
  left: string;
  right: string;
  cls: string;
};

/** 连续的 −/+ 块配对成左右行；上下文两侧同文。 */
export function splitUnified(diff: string): SplitRow[] {
  const lines = diff.replace(/\r\n/g, "\n").split("\n");
  const out: SplitRow[] = [];
  let i = 0;
  while (i < lines.length) {
    const line = lines[i];
    if (isDel(line) || isAdd(line)) {
      const dels: string[] = [];
      const adds: string[] = [];
      while (i < lines.length && (isDel(lines[i]) || isAdd(lines[i]))) {
        if (isDel(lines[i])) dels.push(lines[i]);
        else adds.push(lines[i]);
        i++;
      }
      const n = Math.max(dels.length, adds.length);
      for (let k = 0; k < n; k++) {
        out.push({
          left: dels[k] ?? "",
          right: adds[k] ?? "",
          cls: "sp-chg",
        });
      }
      continue;
    }
    out.push({ left: line, right: line, cls: classify(line) === "dl" ? "sp-ctx" : classify(line) });
    i++;
  }
  return out;
}

export function DiffView({
  diff,
  mode = "unified",
}: {
  diff: string;
  mode?: "unified" | "split";
}) {
  if (mode === "split") {
    const rows = splitUnified(diff);
    return (
      <pre className="diff-view diff-split" aria-label="diff split">
        {rows.map((r, i) => (
          <div key={i} className={`sr ${r.cls}`}>
            <span className="sr-l">{r.left || " "}</span>
            <span className="sr-r">{r.right || " "}</span>
          </div>
        ))}
      </pre>
    );
  }

  const lines = diff.replace(/\r\n/g, "\n").split("\n");
  return (
    <pre className="diff-view" aria-label="diff">
      {lines.map((line, i) => (
        <div key={i} className={classify(line)}>
          {line || " "}
        </div>
      ))}
    </pre>
  );
}
