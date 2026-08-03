#!/usr/bin/env node
/**
 * Generate native-prototype/src/themes_generated.rs from src/themes.ts.
 * No Node runtime dependencies — pure stdlib parsing of TERMINAL_THEMES.
 *
 * Usage (from repo root):
 *   node native-prototype/scripts/generate-themes.mjs
 */

import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const REPO_ROOT = path.resolve(__dirname, '../..');
const SOURCE_PATH = path.join(REPO_ROOT, 'src', 'themes.ts');
const OUTPUT_PATH = path.join(REPO_ROOT, 'native-prototype', 'src', 'themes_generated.rs');

const EXPECTED_COUNT = 191;

const COLOR_FIELDS = [
  'background',
  'foreground',
  'cursor',
  'selection',
  'black',
  'red',
  'green',
  'yellow',
  'blue',
  'magenta',
  'cyan',
  'white',
  'brightBlack',
  'brightRed',
  'brightGreen',
  'brightYellow',
  'brightBlue',
  'brightMagenta',
  'brightCyan',
  'brightWhite',
];

const ANSI_FIELDS = [
  'black',
  'red',
  'green',
  'yellow',
  'blue',
  'magenta',
  'cyan',
  'white',
  'brightBlack',
  'brightRed',
  'brightGreen',
  'brightYellow',
  'brightBlue',
  'brightMagenta',
  'brightCyan',
  'brightWhite',
];

function fail(message) {
  console.error(`generate-themes: ${message}`);
  process.exit(1);
}

function parseHexColor(value, themeName, field) {
  if (typeof value !== 'string') {
    fail(`theme "${themeName}" field "${field}" is not a string`);
  }
  const m = /^#([0-9a-fA-F]{6})$/.exec(value);
  if (!m) {
    fail(`theme "${themeName}" field "${field}" has invalid color "${value}" (expected #RRGGBB)`);
  }
  const n = parseInt(m[1], 16);
  return [(n >> 16) & 0xff, (n >> 8) & 0xff, n & 0xff];
}

function formatRgb(rgb) {
  return `[0x${rgb[0].toString(16).padStart(2, '0')}, 0x${rgb[1].toString(16).padStart(2, '0')}, 0x${rgb[2].toString(16).padStart(2, '0')}]`;
}

function escapeRustString(s) {
  return s
    .replace(/\\/g, '\\\\')
    .replace(/"/g, '\\"')
    .replace(/\n/g, '\\n')
    .replace(/\r/g, '\\r')
    .replace(/\t/g, '\\t');
}

/**
 * Extract field values from a theme object body.
 * Maps TS selectionBackground → selection.
 */
function extractThemeFields(body, themeName) {
  const fields = {};
  const re = /\b(background|foreground|cursor|selectionBackground|selection|black|red|green|yellow|blue|magenta|cyan|white|brightBlack|brightRed|brightGreen|brightYellow|brightBlue|brightMagenta|brightCyan|brightWhite)\s*:\s*'([^']*)'/g;
  let match;
  while ((match = re.exec(body)) !== null) {
    let key = match[1];
    if (key === 'selectionBackground') {
      key = 'selection';
    }
    if (Object.prototype.hasOwnProperty.call(fields, key)) {
      fail(`theme "${themeName}" has duplicate field "${key}"`);
    }
    fields[key] = match[2];
  }
  return fields;
}

function parseThemes(source) {
  const startMarker = 'export const TERMINAL_THEMES';
  const startIdx = source.indexOf(startMarker);
  if (startIdx < 0) {
    fail('TERMINAL_THEMES export not found in src/themes.ts');
  }

  const braceStart = source.indexOf('{', startIdx);
  if (braceStart < 0) {
    fail('TERMINAL_THEMES opening brace not found');
  }

  // Walk braces to find matching close of the Record object.
  let depth = 0;
  let endIdx = -1;
  for (let i = braceStart; i < source.length; i++) {
    const ch = source[i];
    if (ch === '{') depth += 1;
    else if (ch === '}') {
      depth -= 1;
      if (depth === 0) {
        endIdx = i;
        break;
      }
    }
  }
  if (endIdx < 0) {
    fail('TERMINAL_THEMES closing brace not found');
  }

  const recordBody = source.slice(braceStart + 1, endIdx);
  const themes = [];
  const seenNames = new Set();

  // Match each top-level theme entry: 'Name': { ... },
  const entryRe = /'((?:\\'|[^'])*)'\s*:\s*\{/g;
  let entryMatch;
  while ((entryMatch = entryRe.exec(recordBody)) !== null) {
    const name = entryMatch[1].replace(/\\'/g, "'");
    const bodyStart = entryMatch.index + entryMatch[0].length;

    // Find matching closing brace for this theme object.
    let d = 1;
    let j = bodyStart;
    while (j < recordBody.length && d > 0) {
      const c = recordBody[j];
      if (c === '{') d += 1;
      else if (c === '}') d -= 1;
      j += 1;
    }
    if (d !== 0) {
      fail(`unclosed theme object for "${name}"`);
    }
    const body = recordBody.slice(bodyStart, j - 1);

    if (seenNames.has(name)) {
      fail(`duplicate theme name "${name}"`);
    }
    seenNames.add(name);

    const fields = extractThemeFields(body, name);
    for (const field of COLOR_FIELDS) {
      if (!Object.prototype.hasOwnProperty.call(fields, field)) {
        fail(`theme "${name}" missing required field "${field}"`);
      }
    }

    const colors = {};
    for (const field of COLOR_FIELDS) {
      colors[field] = parseHexColor(fields[field], name, field);
    }

    themes.push({ name, colors });
  }

  return themes;
}

function generateRust(themes) {
  const lines = [];
  lines.push('// @generated by native-prototype/scripts/generate-themes.mjs');
  lines.push('// DO NOT EDIT MANUALLY — regenerate from src/themes.ts');
  lines.push('');
  lines.push('use super::TerminalTheme;');
  lines.push('');
  lines.push(`pub static TERMINAL_THEMES: &[TerminalTheme] = &[`);

  for (const theme of themes) {
    const c = theme.colors;
    const ansi = ANSI_FIELDS.map((f) => formatRgb(c[f])).join(', ');
    lines.push('    TerminalTheme {');
    lines.push(`        name: "${escapeRustString(theme.name)}",`);
    lines.push(`        background: ${formatRgb(c.background)},`);
    lines.push(`        foreground: ${formatRgb(c.foreground)},`);
    lines.push(`        cursor: ${formatRgb(c.cursor)},`);
    lines.push(`        selection: ${formatRgb(c.selection)},`);
    lines.push(`        ansi: [${ansi}],`);
    lines.push('    },');
  }

  lines.push('];');
  lines.push('');
  return lines.join('\n');
}

function main() {
  if (!fs.existsSync(SOURCE_PATH)) {
    fail(`source not found: ${SOURCE_PATH}`);
  }

  const source = fs.readFileSync(SOURCE_PATH, 'utf8');
  const themes = parseThemes(source);

  if (themes.length !== EXPECTED_COUNT) {
    fail(`expected ${EXPECTED_COUNT} themes, found ${themes.length}`);
  }

  const rust = generateRust(themes);
  fs.mkdirSync(path.dirname(OUTPUT_PATH), { recursive: true });
  fs.writeFileSync(OUTPUT_PATH, rust, 'utf8');
  console.log(`Wrote ${themes.length} themes to ${path.relative(REPO_ROOT, OUTPUT_PATH)}`);
}

main();
