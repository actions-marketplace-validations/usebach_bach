// Writes a CycloneDX 1.5 software bill of materials for one crate of this
// workspace: the crate itself and every crate it is built from (normal
// dependencies, not dev or build-only tools), with versions, licences and the
// checksums from Cargo.lock.
//
//   bun scripts/sbom.mjs bach-cli dist/bach-cli.cdx.json
//   bun scripts/sbom.mjs bach-setup dist/bach-setup.cdx.json --target x86_64-pc-windows-msvc
//
// The release workflow runs it for both programs and attests the results.

import { createHash } from "node:crypto";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname } from "node:path";
import { spawnSync } from "node:child_process";

const args = process.argv.slice(2);
const flag = (name) => {
  const at = args.indexOf(name);
  if (at < 0) return undefined;
  const [, value] = args.splice(at, 2);
  return value;
};
const target = flag("--target");
const [root, out] = args;
if (!root || !out) {
  console.error("usage: bun scripts/sbom.mjs <crate> <out.json> [--target <triple>]");
  process.exit(2);
}

const metadataArgs = ["metadata", "--format-version", "1", "--locked"];
if (target) metadataArgs.push("--filter-platform", target);
const run = spawnSync("cargo", metadataArgs, { encoding: "utf8", maxBuffer: 1 << 28 });
if (run.status !== 0) {
  console.error(run.stderr);
  process.exit(1);
}
const metadata = JSON.parse(run.stdout);
const packages = new Map(metadata.packages.map((pkg) => [pkg.id, pkg]));
const nodes = new Map(metadata.resolve.nodes.map((node) => [node.id, node]));
const start = metadata.packages.find((pkg) => pkg.name === root && metadata.workspace_members.includes(pkg.id));
if (!start) {
  console.error(`${root} isn't a crate of this workspace`);
  process.exit(1);
}

// What Cargo.lock recorded as each registry crate's SHA-256.
const checksums = new Map();
for (const block of readFileSync("Cargo.lock", "utf8").split("[[package]]").slice(1)) {
  const field = (key) => block.match(new RegExp(`^${key} = "([^"]*)"`, "m"))?.[1];
  if (field("checksum")) checksums.set(`${field("name")}@${field("version")}`, field("checksum"));
}

// Walk normal dependencies only: what ends up in the program.
const seen = new Set([start.id]);
const queue = [start.id];
while (queue.length) {
  const node = nodes.get(queue.pop());
  for (const dep of node?.deps ?? []) {
    if (!dep.dep_kinds.some((kind) => kind.kind === null)) continue;
    if (seen.has(dep.pkg)) continue;
    seen.add(dep.pkg);
    queue.push(dep.pkg);
  }
}

const purl = (pkg) => `pkg:cargo/${pkg.name}@${pkg.version}`;
const component = (pkg) => {
  const sum = checksums.get(`${pkg.name}@${pkg.version}`);
  const license = pkg.license ? pkg.license.replace(/\//g, " OR ") : undefined;
  return {
    type: "library",
    "bom-ref": purl(pkg),
    name: pkg.name,
    version: pkg.version,
    purl: purl(pkg),
    ...(pkg.description ? { description: pkg.description } : {}),
    ...(license ? { licenses: [{ expression: license }] } : {}),
    ...(sum ? { hashes: [{ alg: "SHA-256", content: sum }] } : {}),
    ...(pkg.repository ? { externalReferences: [{ type: "vcs", url: pkg.repository }] } : {}),
  };
};

const components = [...seen]
  .filter((id) => id !== start.id)
  .map((id) => component(packages.get(id)))
  .sort((a, b) => a["bom-ref"].localeCompare(b["bom-ref"]));
const dependencies = [...seen]
  .map((id) => {
    const pkg = packages.get(id);
    const direct = (nodes.get(id)?.deps ?? [])
      .filter((dep) => seen.has(dep.pkg) && dep.dep_kinds.some((kind) => kind.kind === null))
      .map((dep) => purl(packages.get(dep.pkg)));
    return { ref: purl(pkg), dependsOn: [...new Set(direct)].sort() };
  })
  .sort((a, b) => a.ref.localeCompare(b.ref));

const bom = {
  bomFormat: "CycloneDX",
  specVersion: "1.5",
  // A serial number is required by tools that take the file (GitHub's attest
  // action refuses a CycloneDX file without one). It is made from the content,
  // not at random, so the same sources still give the same file.
  serialNumber: "urn:uuid:00000000-0000-0000-0000-000000000000",
  version: 1,
  metadata: {
    // No time stamp: the same sources give the same file.
    tools: { components: [{ type: "application", name: "bach scripts/sbom.mjs" }] },
    component: { ...component(start), type: "application", ...(target ? { properties: [{ name: "target", value: target }] } : {}) },
  },
  components,
  dependencies,
};

{
  const hex = createHash("sha256").update(JSON.stringify(bom)).digest("hex");
  // The 128 bits of a version 5 style UUID: version and variant bits set.
  const variant = ((parseInt(hex[16], 16) & 0x3) | 0x8).toString(16);
  bom.serialNumber = `urn:uuid:${hex.slice(0, 8)}-${hex.slice(8, 12)}-5${hex.slice(13, 16)}-${variant}${hex.slice(17, 20)}-${hex.slice(20, 32)}`;
}

mkdirSync(dirname(out), { recursive: true });
writeFileSync(out, JSON.stringify(bom, null, 2) + "\n");
console.log(`${out}: ${start.name} ${start.version} and ${components.length} crates`);
