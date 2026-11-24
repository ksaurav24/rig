#!/usr/bin/env node
// Validates Rig documentation completeness and link/anchor integrity against the codebase and static export.

import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const docsRoot = path.resolve(__dirname, '..');
const repoRoot = path.resolve(docsRoot, '..');
const contentRoot = path.join(docsRoot, 'content', 'docs');
const outRoot = path.join(docsRoot, 'out');

function walk(dir, ext) {
  const results = [];
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      results.push(...walk(full, ext));
    } else if (!ext || full.endsWith(ext)) {
      results.push(full);
    }
  }
  return results;
}

const mdxFiles = walk(contentRoot, '.mdx');
console.log(`Checking ${mdxFiles.length} MDX documentation pages...`);

const allMdxText = mdxFiles
  .map((f) => fs.readFileSync(f, 'utf8'))
  .join('\n');

// 1. Verify every MCP tool in crates/rig-cli/src/mcp/tools.rs is documented
const mcpToolsSrc = fs.readFileSync(
  path.join(repoRoot, 'crates/rig-cli/src/mcp/tools.rs'),
  'utf8',
);
const toolMatches = [
  ...mcpToolsSrc.matchAll(/\b(?:read|write)\(\s*"([a-z0-9_]+)"/g),
].map((m) => m[1]);

if (toolMatches.length === 0) {
  throw new Error('Could not parse MCP tools from crates/rig-cli/src/mcp/tools.rs');
}

const missingTools = toolMatches.filter((t) => !allMdxText.includes(t));
if (missingTools.length > 0) {
  throw new Error(`Undocumented MCP tools: ${missingTools.join(', ')}`);
}
console.log(`✓ All ${toolMatches.length} MCP tools are documented.`);

// 2. Verify every top-level CLI subcommand is documented
const expectedCliCommands = [
  'rig login',
  'rig logout',
  'rig context',
  'rig link',
  'rig open',
  'rig status',
  'rig projects',
  'rig apps',
  'rig deploy',
  'rig up',
  'rig adopt',
  'rig start',
  'rig restart',
  'rig rollback',
  'rig stop',
  'rig deployments',
  'rig logs',
  'rig exec',
  'rig env',
  'rig domains',
  'rig db',
  'rig backup',
  'rig metrics',
  'rig top',
  'rig tokens',
  'rig members',
  'rig audit',
  'rig alerts',
  'rig webhook',
  'rig github',
  'rig completions',
  'rig mcp',
];
const missingCli = expectedCliCommands.filter((c) => !allMdxText.includes(c));
if (missingCli.length > 0) {
  throw new Error(`Undocumented CLI commands: ${missingCli.join(', ')}`);
}
console.log(`✓ All ${expectedCliCommands.length} CLI commands are documented.`);

// 3. Verify all internal /docs/... links and #anchors resolve in the built static export
if (!fs.existsSync(outRoot)) {
  throw new Error(`Static export directory ${outRoot} does not exist. Run pnpm build first.`);
}

const linkPattern = /(?:href=["']|\]\()(\/docs\/[^"')\s]*)/g;
const errors = [];
let checkedLinks = 0;

for (const file of mdxFiles) {
  const text = fs.readFileSync(file, 'utf8');
  for (const match of text.matchAll(linkPattern)) {
    const rawHref = match[1];
    const [routePart, hashPart] = rawHref.split('#');
    const cleanRoute = routePart.endsWith('/') ? routePart : `${routePart}/`;
    const htmlFile = path.join(outRoot, cleanRoute, 'index.html');
    checkedLinks++;

    if (!fs.existsSync(htmlFile)) {
      errors.push(`${path.relative(docsRoot, file)}: broken route link "${rawHref}" (missing ${path.relative(docsRoot, htmlFile)})`);
      continue;
    }

    if (hashPart) {
      const html = fs.readFileSync(htmlFile, 'utf8');
      if (!html.includes(`id="${hashPart}"`)) {
        errors.push(`${path.relative(docsRoot, file)}: broken anchor "#${hashPart}" in "${rawHref}"`);
      }
    }
  }
}

if (errors.length > 0) {
  console.error('Link validation errors:\n' + errors.join('\n'));
  process.exit(1);
}

console.log(`✓ All ${checkedLinks} internal /docs/... links and anchors resolved cleanly in out/.`);
