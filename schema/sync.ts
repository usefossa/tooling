import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, resolve } from "node:path";

const files = ["Cargo.toml", "schema.json", "fixtures.json", "src/lib.rs", "src/icon.rs"];
const args = process.argv.slice(2), write = args[0] === "--write";
if (!["--write", "--check"].includes(args.shift() ?? "") || !args.length || args.length % 2) throw Error("Usage: bun schema/sync.ts --check|--write --rust PATH --frontend PATH --indexer PATH --client PATH --counter PATH");
for (let index = 0; index < args.length; index += 2) {
  const kind = args[index]!, root = resolve(args[index + 1]!);
  if (!["--rust", "--frontend", "--indexer", "--client", "--counter"].includes(kind)) throw Error(`Unknown target ${kind}`);
  const targets = kind === "--client" ? [["../sdk/text-counter/core.js", "opencode/src/inference/text-counter-core.js"], ["../sdk/text-counter/fixture.json", "opencode/tests/fixtures/text-counter.json"]]
    : kind === "--counter" ? ["Cargo.toml", "src/lib.rs", "core.js", "fixture.json", "jinja-0.5.13.cjs", "tokenizers-0.2.0.cjs", "LICENSE-jinja", "LICENSE-tokenizers"].map(file => [`../sdk/text-counter/${file}`, `vendor/text-counter/${file}`])
    : kind === "--frontend" ? [["../sdk/text-counter/core.js", "lib/inference/text-counter-core.js"], ["schema.json", "public/schemas/metadata/v1.json"], ["fixtures.json", "public/schemas/metadata/v1.cases.json"]] : kind === "--indexer" ? [["schema.json", "indexer/src/metadata/schema.json"], ["fixtures.json", "indexer/src/metadata/fixtures.json"], ["src/lib.rs", "indexer/src/metadata/schema.rs"], ["src/icon.rs", "indexer/src/metadata/icon.rs"]] : files.map(file => [file, `vendor/fossa-metadata/${file}`]);
  for (const [source, destination] of targets) {
    let bytes = readFileSync(resolve(import.meta.dir, source!));
    if (kind === "--indexer" && source === "src/lib.rs") bytes = Buffer.from(bytes.toString().replaceAll('include_str!("../', 'include_str!("'));
    const path = resolve(root, destination!);
    if (write) { mkdirSync(dirname(path), { recursive: true }); writeFileSync(path, bytes); }
    else if (!readFileSync(path).equals(bytes)) throw Error(`Metadata artifact differs: ${path}`);
  }
}
console.log(`Metadata artifacts ${write ? "synchronized" : "match canonical source"}.`);
