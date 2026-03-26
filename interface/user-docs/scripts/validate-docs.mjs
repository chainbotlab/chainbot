import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { resolve } from "node:path";

const projectRoot = process.cwd();
const docsConfigPath = resolve(projectRoot, "docs.json");
const docsConfig = JSON.parse(await readFile(docsConfigPath, "utf8"));

assert.equal(typeof docsConfig.theme, "string", "docs.json must define a theme");
assert.equal(typeof docsConfig.name, "string", "docs.json must define a project name");
assert.equal(typeof docsConfig.colors?.primary, "string", "docs.json must define colors.primary");
assert.ok(Array.isArray(docsConfig.navigation?.pages), "docs.json must define navigation.pages");

for (const page of docsConfig.navigation.pages) {
  assert.equal(typeof page, "string", `navigation entry must be a string: ${page}`);
  const pagePath = resolve(projectRoot, `${page}.mdx`);
  const source = await readFile(pagePath, "utf8");
  assert.match(source, /^---[\s\S]*title:/m, `${page}.mdx must define a title in frontmatter`);
  assert.match(source, /^---[\s\S]*description:/m, `${page}.mdx must define a description in frontmatter`);
}

console.log("Mintlify docs structural checks passed.");
