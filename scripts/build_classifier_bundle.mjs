#!/usr/bin/env node

/**
 * Build and verify script for Argos Natural Bayes QuickJS bundle.
 *
 * Usage:
 *   node scripts/build_classifier_bundle.mjs          # Build bundle, patch, and manifest
 *   node scripts/build_classifier_bundle.mjs --check  # Verify committed bundle and upstream integrity
 */

import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import { fileURLToPath } from 'node:url';
import esbuild from 'esbuild';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const REPO_ROOT = path.resolve(__dirname, '..');

const ASSETS_DIR = path.join(
  REPO_ROOT,
  'crates',
  'argos-osint-core',
  'assets',
  'classifier'
);
const UPSTREAM_DIR = path.join(ASSETS_DIR, 'upstream');
const SHIMS_DIR = path.join(ASSETS_DIR, 'shims');
const BUNDLE_OUT = path.join(ASSETS_DIR, 'natural-classifier.js');
const MANIFEST_OUT = path.join(ASSETS_DIR, 'source-manifest.json');
const PATCH_OUT = path.join(ASSETS_DIR, 'natural-bayes.patch');
const IDENTITY_OUT = path.join(ASSETS_DIR, 'bundle-identity.rs');
const ADAPTER_FILE = path.join(ASSETS_DIR, 'natural-bayes.adapt.js');

const SCRATCH_DIR = path.join(
  REPO_ROOT,
  '.agent-scratch',
  'classifier-build-input'
);

const UPSTREAM_CLOSURE_FILES = [
  'natural/lib/natural/classifiers/bayes_classifier.js',
  'natural/lib/natural/classifiers/classifier.js',
  'natural/lib/natural/stemmers/porter_stemmer.js',
  'natural/lib/natural/stemmers/stemmer.js',
  'natural/lib/natural/tokenizers/aggressive_tokenizer.js',
  'natural/lib/natural/tokenizers/tokenizer.js',
  'natural/lib/natural/util/stopwords.js',
  'natural/lib/natural/ngrams/ngrams.js',
  'natural/lib/natural/tokenizers/regexp_tokenizer.js',
  'natural/lib/natural/tokenizers/orthography_matchers.js',
  'natural/lib/natural/tfidf/tfidf.js',
  'natural/lib/natural/distance/dice_coefficient.js',
  'natural/lib/natural/distance/jaro-winkler_distance.js',
  'underscore/underscore-node-f.cjs',
  'underscore/LICENSE',
  'apparatus/lib/apparatus/classifier/bayes_classifier.js',
  'apparatus/lib/apparatus/classifier/classifier.js',
];

function sha256(data) {
  return crypto.createHash('sha256').update(data).digest('hex');
}

/**
 * Simple unified diff implementation in JS.
 */
function makeUnifiedDiff(oldPath, newPath, oldContent, newContent) {
  const oldLines = oldContent.split('\n');
  const newLines = newContent.split('\n');

  if (oldContent === newContent) {
    return '';
  }

  // Quick LCS or line difference for clean patch
  let header = `--- ${oldPath}\n+++ ${newPath}\n`;
  let diff = '';

  let i = 0;
  let j = 0;
  while (i < oldLines.length || j < newLines.length) {
    if (i < oldLines.length && j < newLines.length && oldLines[i] === newLines[j]) {
      i++;
      j++;
    } else {
      let hunkOldStart = Math.max(0, i - 3);
      let hunkNewStart = Math.max(0, j - 3);
      let contextBefore = [];
      for (let k = hunkOldStart; k < i; k++) {
        contextBefore.push(' ' + oldLines[k]);
      }

      // Collect changes
      let changedOld = [];
      let changedNew = [];

      // Find next synchronization point within 20 lines
      let syncOld = -1;
      let syncNew = -1;
      outer: for (let d = 1; d <= 25; d++) {
        for (let di = 0; di <= d; di++) {
          let dj = d - di;
          let testI = i + di;
          let testJ = j + dj;
          if (
            testI < oldLines.length &&
            testJ < newLines.length &&
            oldLines[testI] === newLines[testJ] &&
            (testI + 1 >= oldLines.length ||
              testJ + 1 >= newLines.length ||
              oldLines[testI + 1] === newLines[testJ + 1])
          ) {
            syncOld = testI;
            syncNew = testJ;
            break outer;
          }
        }
      }

      if (syncOld === -1 || syncNew === -1) {
        while (i < oldLines.length) changedOld.push('-' + oldLines[i++]);
        while (j < newLines.length) changedNew.push('+' + newLines[j++]);
      } else {
        while (i < syncOld) changedOld.push('-' + oldLines[i++]);
        while (j < syncNew) changedNew.push('+' + newLines[j++]);
      }

      let contextAfter = [];
      let afterLimit = Math.min(3, oldLines.length - i, newLines.length - j);
      for (let k = 0; k < afterLimit; k++) {
        contextAfter.push(' ' + oldLines[i + k]);
      }
      i += afterLimit;
      j += afterLimit;

      let hunkOldCount = contextBefore.length + changedOld.length + contextAfter.length;
      let hunkNewCount = contextBefore.length + changedNew.length + contextAfter.length;

      diff += `@@ -${hunkOldStart + 1},${hunkOldCount} +${hunkNewStart + 1},${hunkNewCount} @@\n`;
      for (const line of contextBefore) diff += line + '\n';
      for (const line of changedOld) diff += line + '\n';
      for (const line of changedNew) diff += line + '\n';
      for (const line of contextAfter) diff += line + '\n';
    }
  }

  return header + diff;
}

async function main() {
  const isCheckMode = process.argv.includes('--check');

  // Verify all upstream files exist
  const upstreamFilesMeta = [];
  for (const relPath of UPSTREAM_CLOSURE_FILES) {
    const fullPath = path.join(UPSTREAM_DIR, relPath);
    if (!fs.existsSync(fullPath)) {
      console.error(`Missing required upstream file: ${fullPath}`);
      process.exit(1);
    }
    const content = fs.readFileSync(fullPath);
    upstreamFilesMeta.push({
      path: relPath,
      sha256: sha256(content),
      bytes: content.length,
    });
  }

  // Load adapter
  if (!fs.existsSync(ADAPTER_FILE)) {
    console.error(`Missing adapter file: ${ADAPTER_FILE}`);
    process.exit(1);
  }
  const { adaptSource } = await import(ADAPTER_FILE);
  const adapterBytes = fs.readFileSync(ADAPTER_FILE);
  const adapterSha = sha256(adapterBytes);

  // Prepare working directory in scratch
  fs.mkdirSync(SCRATCH_DIR, { recursive: true });

  let combinedPatch = '';

  for (const relPath of UPSTREAM_CLOSURE_FILES) {
    const srcPath = path.join(UPSTREAM_DIR, relPath);
    const destPath = path.join(SCRATCH_DIR, relPath);
    fs.mkdirSync(path.dirname(destPath), { recursive: true });

    const rawContent = fs.readFileSync(srcPath, 'utf8');
    const adaptedContent = adaptSource(relPath, rawContent);
    fs.writeFileSync(destPath, adaptedContent, 'utf8');

    if (rawContent !== adaptedContent) {
      const fileDiff = makeUnifiedDiff(
        `a/${relPath}`,
        `b/${relPath}`,
        rawContent,
        adaptedContent
      );
      combinedPatch += fileDiff + '\n';
    }
  }

  // Also copy shims into scratch so relative paths resolve cleanly
  const scratchShimsDir = path.join(SCRATCH_DIR, 'shims');
  fs.mkdirSync(scratchShimsDir, { recursive: true });
  for (const shim of fs.readdirSync(SHIMS_DIR)) {
    const src = path.join(SHIMS_DIR, shim);
    if (fs.statSync(src).isFile()) {
      fs.copyFileSync(src, path.join(scratchShimsDir, shim));
    }
  }

  // Write entry point in scratch
  const entryPath = path.join(SCRATCH_DIR, 'entry.js');
  const entryCode = `'use strict';
const BayesClassifier = require('./natural/lib/natural/classifiers/bayes_classifier');
const PorterStemmer = require('./natural/lib/natural/stemmers/porter_stemmer');
const AggressiveTokenizer = require('./natural/lib/natural/tokenizers/aggressive_tokenizer');
const NGrams = require('./natural/lib/natural/ngrams/ngrams');
const RegexpTokenizer = require('./natural/lib/natural/tokenizers/regexp_tokenizer').RegexpTokenizer;
const TfIdf = require('./natural/lib/natural/tfidf/tfidf');
const DiceCoefficient = require('./natural/lib/natural/distance/dice_coefficient');
const JaroWinklerDistance = require('./natural/lib/natural/distance/jaro-winkler_distance');

module.exports = {
  BayesClassifier,
  PorterStemmer,
  AggressiveTokenizer, NGrams, RegexpTokenizer, TfIdf, DiceCoefficient, JaroWinklerDistance
};
`;
  fs.writeFileSync(entryPath, entryCode, 'utf8');

  // Bundle with esbuild
  const buildResult = await esbuild.build({
    entryPoints: [entryPath],
    bundle: true,
    format: 'iife',
    globalName: 'argosNaturalBayes',
    platform: 'neutral',
    target: 'es2020',
    minify: true,
    sourcemap: false,
    legalComments: 'none',
    define: {
      'process.env.NODE_ENV': '"production"',
    },
    alias: {
      underscore: path.join(SCRATCH_DIR, 'underscore/underscore-node-f.cjs'),
      events: path.join(SHIMS_DIR, 'events-shim.js'),
      util: path.join(SHIMS_DIR, 'util-shim.js'),
      fs: path.join(SHIMS_DIR, 'fs-shim.js'),
      os: path.join(SHIMS_DIR, 'os-shim.js'),
      'webworker-threads': path.join(SHIMS_DIR, 'webworker-shim.js'),
      apparatus: path.join(
        SCRATCH_DIR,
        'apparatus',
        'lib',
        'apparatus',
        'classifier',
        'bayes_classifier.js'
      ),
    },
    banner: {
      js: '/* Argos Bayes classifier bundle — natural@8.2.6 apparatus@0.0.10 — see assets/classifier/source-manifest.json */',
    },
    footer: {
      js: 'globalThis.argosNaturalBayes = argosNaturalBayes;',
    },
    write: false,
  });

  const bundledCode = buildResult.outputFiles[0].text;
  const bundleBytes = Buffer.byteLength(bundledCode, 'utf8');
  const bundleSha = sha256(Buffer.from(bundledCode, 'utf8'));

  // Security & cleanliness assertion on bundle
  if (bundledCode.includes('process.env')) {
    console.error('Bundle contains process.env');
    process.exit(1);
  }
  if (bundledCode.includes('require(')) {
    console.error('Bundle contains unbundled require() call');
    process.exit(1);
  }
  if (bundledCode.includes('trainParallel')) {
    console.error('Bundle contains excluded trainParallel');
    process.exit(1);
  }

  const rustIdentity = `// Generated by scripts/build_classifier_bundle.mjs.\npub const COMPILED_BUNDLE_SHA256: &str =\n    "${bundleSha}";\n`;
  const patchSha = sha256(Buffer.from(combinedPatch, 'utf8'));

  const manifest = {
    upstream: {
      natural: {
        version: '8.2.6',
        gitHead: 'e5bb63d6af741209e71691ae8702e91664d61c29',
        tarball_sha256:
          '05094a3b7c6750e942b943758288ca193249a96cc1105f25c4eeb35c2df60040',
        integrity:
          'sha512-NEZgLOCFZVWpzEVaDz+ZyPhWGZQKHS2Gmxqzex5/WVgwkkzUDodZ2fFyqeOXpxIbxnYsvkRc+RuDkV+hAdzVRg==',
      },
      underscore: {version:'1.13.8',license_file:'upstream/underscore/LICENSE'},
      apparatus: {
        version: '0.0.10',
        gitHead: '861cf4cbb0775e2fd12ffaf30d01b7d8f9497e12',
        tarball_sha256:
          '4bc0726d881db4dadab338f7cb2eb2dc1f2b9adc682263753d851daeb80c58ee',
        integrity:
          'sha512-KLy/ugo33KZA7nugtQ7O0E1c8kQ52N3IvD/XgIh4w/Nr28ypfkwDfA67F1ev4N1m5D+BOk1+b2dEJDfpj/VvZg==',
      },
    },
    build: {
      esbuild_version: esbuild.version,
      node_version: process.version,
      bundle_file: 'natural-classifier.js',
      bundle_sha256: bundleSha,
      bundle_bytes: bundleBytes,
    },
    adaptation: {
      patch_file: 'natural-bayes.patch',
      patch_sha256: patchSha,
      adapter_file: 'natural-bayes.adapt.js',
      adapter_sha256: adapterSha,
      transform_version: 'v2-feature-index',
    },
    vocabulary: {version:'natural-en-phrases-v2',sha256:sha256(fs.readFileSync(path.join(REPO_ROOT,'crates/argos-osint-core/src/classifier/vocabulary.js')))},
    features: { version: 'natural-en-phrases-v2', sha256: sha256(fs.readFileSync(path.join(REPO_ROOT, 'crates/argos-osint-core/src/classifier/features.js'))) },
    bridge: { version: 'argos-classifier-bridge-v2', sha256: sha256(fs.readFileSync(path.join(REPO_ROOT, 'crates/argos-osint-core/src/classifier/bridge.js'))) },
    files: upstreamFilesMeta,
  };

  const manifestJson = JSON.stringify(manifest, null, 2) + '\n';

  if (isCheckMode) {
    // Check mode
    if (!fs.existsSync(IDENTITY_OUT) || fs.readFileSync(IDENTITY_OUT,'utf8') !== rustIdentity) throw new Error('Compiled Rust bundle identity is stale');
    if (!fs.existsSync(BUNDLE_OUT)) {
      console.error(`Check failed: ${BUNDLE_OUT} does not exist`);
      process.exit(1);
    }
    const currentBundleBytes = fs.readFileSync(BUNDLE_OUT);
    const currentBundleSha = sha256(currentBundleBytes);
    if (currentBundleSha !== bundleSha) {
      console.error(
        `Check failed: bundle SHA-256 mismatch.\n  Committed: ${currentBundleSha}\n  Built:     ${bundleSha}`
      );
      process.exit(1);
    }
    if (currentBundleBytes.length !== bundleBytes) {
      console.error(
        `Check failed: bundle byte length mismatch.\n  Committed: ${currentBundleBytes.length}\n  Built:     ${bundleBytes}`
      );
      process.exit(1);
    }

    if (!fs.existsSync(MANIFEST_OUT)) {
      console.error(`Check failed: ${MANIFEST_OUT} does not exist`);
      process.exit(1);
    }
    const currentManifest = JSON.parse(fs.readFileSync(MANIFEST_OUT, 'utf8'));
    if (currentManifest.build.bundle_sha256 !== bundleSha) {
      console.error('Check failed: manifest bundle_sha256 does not match built bundle');
      process.exit(1);
    }
    if (currentManifest.adaptation.adapter_sha256 !== adapterSha || currentManifest.adaptation.patch_sha256 !== patchSha || fs.readFileSync(PATCH_OUT, 'utf8') !== combinedPatch || JSON.stringify(currentManifest.bridge) !== JSON.stringify(manifest.bridge) || JSON.stringify(currentManifest.features) !== JSON.stringify(manifest.features) || JSON.stringify(currentManifest.vocabulary)!==JSON.stringify(manifest.vocabulary) || currentManifest.files.length !== upstreamFilesMeta.length) {
      throw new Error('Check failed: adapter, patch, bridge or upstream closure integrity mismatch');
    }
    for (const f of currentManifest.files) {
      const match = upstreamFilesMeta.find((u) => u.path === f.path);
      if (!match || match.sha256 !== f.sha256) {
        console.error(`Check failed: upstream file ${f.path} checksum mismatch`);
        process.exit(1);
      }
    }

    console.log(
      `Classifier bundle check PASSED (${bundleBytes} bytes, SHA-256: ${bundleSha})`
    );
    process.exit(0);
  }

  // Write outputs
  fs.writeFileSync(IDENTITY_OUT, rustIdentity, 'utf8');
  fs.writeFileSync(BUNDLE_OUT, bundledCode, 'utf8');
  fs.writeFileSync(PATCH_OUT, combinedPatch, 'utf8');
  fs.writeFileSync(MANIFEST_OUT, manifestJson, 'utf8');

  console.log(`Bundle built successfully:`);
  console.log(`  File:    ${BUNDLE_OUT}`);
  console.log(`  Bytes:   ${bundleBytes}`);
  console.log(`  SHA-256: ${bundleSha}`);
  console.log(`  Patch:   ${PATCH_OUT}`);
  console.log(`  Manifest: ${MANIFEST_OUT}`);
}

main().catch((err) => {
  console.error('Bundle build failed:', err);
  process.exit(1);
});
