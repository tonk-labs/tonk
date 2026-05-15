import { useContext, useEffect, useState } from "react";
import { RepoContext } from "../context";
import { BUILTIN_ARTIFACTS, isBuiltin } from "../lib/builtins";

type Claim = { the: string; of: string; is: string };

function toYaml(claims: Claim[]): string {
  if (!claims.length) return "claims: []\n";
  const lines: string[] = ["claims:"];
  for (const claim of claims) {
    lines.push(`  - the: ${claim.the}`);
    lines.push(`    of: ${claim.of}`);
    if (claim.is.includes("\n")) {
      lines.push(`    is: |`);
      for (const line of claim.is.split("\n")) {
        lines.push(`      ${line}`);
      }
    } else {
      lines.push(`    is: ${JSON.stringify(claim.is)}`);
    }
  }
  return lines.join("\n");
}

function builtinClaims(entity: string): Claim[] {
  const b = BUILTIN_ARTIFACTS.find((a) => a.entity === entity);
  if (!b) return [];
  return [
    { the: "dialog.meta/name", of: entity, is: b.name },
    { the: "text/html", of: entity, is: b.html },
  ];
}

type Props = {
  entity: string;
  branch: string;
  onClose: () => void;
};

export function SourcePanel({ entity, branch, onClose }: Props) {
  const repo = useContext(RepoContext);
  const [yaml, setYaml] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (isBuiltin(entity)) {
      setYaml(toYaml(builtinClaims(entity)));
      return;
    }
    if (!repo) return;
    const url = `/api/repository/${encodeURIComponent(repo)}/branch/${encodeURIComponent(branch)}/claim/select?of=${encodeURIComponent(entity)}`;
    fetch(url)
      .then((r) => {
        if (!r.ok) throw new Error(`HTTP ${r.status}`);
        return r.json() as Promise<{ claims: Claim[] }>;
      })
      .then((data) => setYaml(toYaml(data.claims ?? [])))
      .catch((e) => setError(e instanceof Error ? e.message : String(e)));
  }, [entity, branch, repo]);

  return (
    <div className="source-panel" onMouseDown={(e) => e.stopPropagation()}>
      <div className="source-panel__bar">
        <span className="source-panel__title">Source</span>
        <button className="source-panel__close" onClick={onClose} aria-label="close source">
          ✕
        </button>
      </div>
      <div className="source-panel__body">
        {error && <div className="source-panel__error">{error}</div>}
        {!error && yaml === null && (
          <div className="source-panel__loading">Loading…</div>
        )}
        {yaml !== null && <pre className="source-panel__pre">{yaml}</pre>}
      </div>
    </div>
  );
}
