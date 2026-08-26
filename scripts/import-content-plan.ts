#!/usr/bin/env npx tsx
// import-content-plan — Valet (v1.28.42): turn the marketing plan's 100-title
// register (CSV: title,pillar,due) into one `valet/reminder` run per planned
// post. Talks ONLY to the public HTTP surface (`POST /workflow/runs`), never
// the DB. Idempotent per (title, due) by construction: the server dedupes
// nothing here, so re-running the SAME CSV file creates duplicates — pass
// `--dry-run` to inspect first (house discipline).
//
// Usage:
//   scripts/import-content-plan.ts plan.csv [--domain personal] [--dry-run]
//         [--base-url http://127.0.0.1:8765]
//
// CSV header required: title,pillar,due   (due = YYYY-MM-DD; reminders fire
// at 09:00 local on the due date).

import { readFileSync } from "node:fs";

interface Row {
  title: string;
  pillar: string;
  due: string;
}

function parseArgs(): { file: string; domain: string; dryRun: boolean; baseUrl: string } {
  const argv = process.argv.slice(2);
  const file = argv[0];
  if (!file || file.startsWith("--")) {
    console.error("usage: import-content-plan.ts <plan.csv> [--domain D] [--dry-run] [--base-url U]");
    process.exit(2);
  }
  const flag = (name: string): string | undefined => {
    const i = argv.indexOf(name);
    return i >= 0 ? argv[i + 1] : undefined;
  };
  return {
    file,
    domain: flag("--domain") ?? "personal",
    dryRun: argv.includes("--dry-run"),
    baseUrl: flag("--base-url") ?? process.env.BRAIN_URL ?? "http://127.0.0.1:8765",
  };
}

function parseCsv(text: string): Row[] {
  const lines = text.split(/\r?\n/).filter((l) => l.trim().length > 0);
  if (lines.length < 2) throw new Error("CSV needs a header row + at least one data row");
  const header = lines[0].split(",").map((h) => h.trim().toLowerCase());
  const idx = (name: string) => {
    const i = header.indexOf(name);
    if (i < 0) throw new Error(`CSV header must contain '${name}'`);
    return i;
  };
  const [ti, pi, di] = [idx("title"), idx("pillar"), idx("due")];
  return lines.slice(1).map((line) => {
    const cols = line.split(",").map((c) => c.trim());
    const row = { title: cols[ti], pillar: cols[pi], due: cols[di] };
    if (!row.title || !/^\d{4}-\d{2}-\d{2}$/.test(row.due)) {
      throw new Error(`bad row: ${line}`);
    }
    return row;
  });
}

function token(): string | undefined {
  // Same ladder as the brain CLI: BRAIN_TOKEN_FILE → BRAIN_TOKEN → default file.
  const fs = require("node:fs") as typeof import("node:fs");
  const path = process.env.BRAIN_TOKEN_FILE ?? `${process.env.HOME}/.config/brain-server/auth-token`;
  try {
    return fs.readFileSync(path, "utf8").split(/\s+/)[0] || undefined;
  } catch {
    return process.env.BRAIN_TOKEN ?? undefined;
  }
}

async function main(): Promise<void> {
  const args = parseArgs();
  const rows = parseCsv(readFileSync(args.file, "utf8"));
  console.log(`import-content-plan: ${rows.length} rows${args.dryRun ? " (dry-run)" : ""}`);
  const bearer = token();
  let created = 0;
  for (const r of rows) {
    const dueAt = Math.floor(new Date(`${r.due}T09:00:00`).getTime() / 1000);
    const state = {
      what: `draft ${r.pillar} post: ${r.title}`.slice(0, 500),
      due_at: dueAt,
      repeat: "none",
      channel: "signal",
      fire_count: 0,
      sla_deadline: dueAt,
    };
    if (args.dryRun) {
      console.log(`  would create: ${state.what} @ ${r.due}`);
      continue;
    }
    const res = await fetch(`${args.baseUrl}/workflow/runs`, {
      method: "POST",
      headers: {
        "content-type": "application/json",
        ...(bearer ? { authorization: `Bearer ${bearer}` } : {}),
      },
      body: JSON.stringify({
        domain: args.domain,
        kind: "valet/reminder",
        state_json: JSON.stringify(state),
      }),
    });
    if (!res.ok) {
      console.error(`  FAILED (${res.status}) for ${r.title}: ${await res.text()}`);
      process.exit(1);
    }
    created += 1;
  }
  if (!args.dryRun) console.log(`created ${created} valet/reminder runs`);
}

main().catch((e) => {
  console.error(`import-content-plan: ${e}`);
  process.exit(1);
});
