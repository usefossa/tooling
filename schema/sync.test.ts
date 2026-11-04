import { expect, test } from "bun:test";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

test("indexer schema sync preserves canonical validator with local include paths", () => {
  const root = mkdtempSync(join(tmpdir(), "fossa-schema-"));
  const run = (mode: string) => Bun.spawnSync([process.execPath, join(import.meta.dir, "sync.ts"), mode, "--indexer", root]);
  try {
    expect(run("--write").exitCode).toBe(0);
    expect(run("--check").exitCode).toBe(0);
    const validator = join(root, "indexer/src/metadata/schema.rs");
    expect(readFileSync(validator, "utf8")).toBe(readFileSync(join(import.meta.dir, "src/lib.rs"), "utf8").replaceAll('include_str!("../', 'include_str!("'));
    expect(readFileSync(join(root, "indexer/src/metadata/icon.rs"), "utf8")).toBe(readFileSync(join(import.meta.dir, "src/icon.rs"), "utf8"));
    writeFileSync(validator, "changed");
    expect(run("--check").exitCode).not.toBe(0);
  } finally { rmSync(root, { recursive: true, force: true }); }
});

test("rust schema sync preserves canonical artifacts for any vendored consumer", () => {
  const root = mkdtempSync(join(tmpdir(), "fossa-schema-"));
  const run = (mode: string) => Bun.spawnSync([process.execPath, join(import.meta.dir, "sync.ts"), mode, "--rust", root]);
  try {
    expect(run("--write").exitCode).toBe(0);
    expect(run("--check").exitCode).toBe(0);
    for (const file of ["Cargo.toml", "schema.json", "fixtures.json", "src/lib.rs", "src/icon.rs"]) {
      expect(readFileSync(join(root, "vendor/fossa-metadata", file))).toEqual(readFileSync(join(import.meta.dir, file)));
    }
    writeFileSync(join(root, "vendor/fossa-metadata/src/lib.rs"), "changed");
    expect(run("--check").exitCode).not.toBe(0);
  } finally { rmSync(root, { recursive: true, force: true }); }
});
