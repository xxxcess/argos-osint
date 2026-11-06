#!/usr/bin/env node

/**
 * Node-vs-bundle parity oracle for Argos Natural Bayes QuickJS bundle.
 *
 * Verifies:
 * 1. Identical tokenization arrays (tokenizeAndStem with keepStops=true)
 * 2. Identical binary feature vectors (textToFeatures)
 * 3. Identical append-only feature order behavior
 * 4. Finite log scores matching within parity tolerance: <= 1e-9 preferred, <= 1e-6 maximum
 * 5. Absence of unresolved require() and forbidden Node-only globals in bundle
 * 6. Finite scoring on long and out-of-vocabulary (OOV) inputs without NaN/Infinity
 */

import fs from 'node:fs';
import path from 'node:path';
import assert from 'node:assert';
import { fileURLToPath } from 'node:url';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const REPO_ROOT = path.resolve(__dirname, '..');

const BUNDLE_PATH = path.join(
  REPO_ROOT,
  'crates',
  'argos-osint-core',
  'assets',
  'classifier',
  'natural-classifier.js'
);

const jsonMode = process.argv.includes('--json') || process.argv.includes('--emit-oracle');
const log = (...args) => { if (!jsonMode) console.log(...args); };

async function main() {
  log('Running classifier bundle parity tests...');

  // 1. Check bundle file exists
  assert.ok(fs.existsSync(BUNDLE_PATH), `Bundle file not found at ${BUNDLE_PATH}`);
  const bundleCode = fs.readFileSync(BUNDLE_PATH, 'utf8');

  // 2. Static security checks on bundle
  log('  1. Checking bundle static hygiene...');
  assert.ok(!bundleCode.includes('process.env'), 'Bundle must not contain process.env');
  assert.ok(!bundleCode.includes('require('), 'Bundle must not contain unbundled require()');
  assert.ok(!bundleCode.includes('trainParallel'), 'Bundle must not contain trainParallel');
  assert.ok(!bundleCode.includes('webworker-threads'), 'Bundle must not contain webworker-threads');

  // 3. Import upstream Node packages
  log('  2. Loading upstream natural and apparatus...');
  const naturalModule = await import('natural');
  const natural = naturalModule.default || naturalModule;

  // 4. Load bundle into isolated global scope
  log('  3. Evaluating bundle to instantiate globalThis.argosNaturalBayes...');
  const sandbox = {};
  const evaluate = new Function('globalThis', bundleCode);
  evaluate(sandbox);

  assert.ok(
    sandbox.argosNaturalBayes,
    'Bundle must export globalThis.argosNaturalBayes'
  );
  const { BayesClassifier: BundleBayes, PorterStemmer: BundleStemmer } =
    sandbox.argosNaturalBayes;
  assert.ok(typeof BundleBayes === 'function', 'BundleBayes must be a constructor');
  assert.ok(
    typeof BundleStemmer.tokenizeAndStem === 'function',
    'BundleStemmer must have tokenizeAndStem'
  );

  // 5. Test tokenization parity
  log('  4. Testing tokenization and stemming parity...');
  const tokenizationCases = [
    'The quick brown fox jumps over 12 lazy dogs!',
    'Sanctions imposed on commercial maritime trade entities in 2026.',
    'Cyber cyber cyber-attacks on electrical power grids cannot be ruled out.',
    'Diplomatic negotiations failed; military forces mobilised near borders.',
    'Environmental pollution and greenhouse emissions increased by 4.5%.',
    'AI-driven neural network capabilities accelerate bioinformatics research.',
    '', // empty
    'A', // single char
    'cannot never neither nor without 12345', // stop words and digits with keepStops=true
  ];

  for (const text of tokenizationCases) {
    const upstreamTokens = natural.PorterStemmer.tokenizeAndStem(text, true);
    const bundleTokens = BundleStemmer.tokenizeAndStem(text, true);
    assert.deepStrictEqual(
      bundleTokens,
      upstreamTokens,
      `Tokenization mismatch for input: "${text}"`
    );
  }

  // 6. Test training and feature order parity
  log('  5. Testing training and binary feature extraction parity...');
  const upstreamClassifier = new natural.BayesClassifier(natural.PorterStemmer);
  upstreamClassifier.setOptions({ keepStops: true });
  const bundleClassifier = new BundleBayes(BundleStemmer);
  bundleClassifier.setOptions({ keepStops: true });

  const trainingCorpus = [
    { text: 'Government legislature approves national cybersecurity budget and policies.', label: 'present' },
    { text: 'Fiscal quarterly inflation rate decelerates across domestic retail banking.', label: 'absent' },
    { text: 'Naval fleet conducts joint maritime patrol exercise in territorial straits.', label: 'present' },
    { text: 'Agricultural commodity grain export yields rise following seasonal harvest.', label: 'absent' },
    { text: 'Satellite intelligence identifies fortified anti-aircraft missile silos.', label: 'present' },
    { text: 'Municipal public transit authority orders zero-emission electric buses.', label: 'absent' },
    { text: 'Strategic diplomatic treaty signed to enhance regional cyber resilience.', label: 'present' },
    { text: 'Commercial air carrier schedules additional intercontinental holiday flights.', label: 'absent' },
  ];

  for (const doc of trainingCorpus) {
    upstreamClassifier.addDocument(doc.text, doc.label);
    bundleClassifier.addDocument(doc.text, doc.label);
  }

  // Train both
  upstreamClassifier.train();
  bundleClassifier.train();

  // Test feature extraction parity across unseen test phrases
  const testPhrases = [
    'National cybersecurity policies under government review',
    'Retail commercial banking inflation yields',
    'Naval missile defence patrol near territorial waters',
    'Transit authority approves public electric budget',
    'Completely unrelated sentence with unknown vocabulary words',
  ];

  for (const phrase of testPhrases) {
    const upstreamFeat = upstreamClassifier.textToFeatures(phrase);
    const bundleFeat = bundleClassifier.textToFeatures(phrase);
    assert.strictEqual(
      bundleFeat.length,
      upstreamFeat.length,
      `Feature vector length mismatch for "${phrase}"`
    );
    assert.deepStrictEqual(bundleFeat, upstreamFeat, `Binary presence feature mismatch for "${phrase}"`);
  }

  // 7. Test log-space score vs upstream probabilityOfClass parity
  log('  6. Testing log-space score parity (tolerance <= 1e-9)...');
  let maxDelta = 0;
  const fixtures = [];

  for (const phrase of testPhrases) {
    const upstreamFeat = upstreamClassifier.textToFeatures(phrase);
    const bundleFeat = bundleClassifier.textToFeatures(phrase);

    for (const label of ['present', 'absent']) {
      const upstreamProb = upstreamClassifier.classifier.probabilityOfClass(
        upstreamFeat,
        label
      );
      const bundleLog = bundleClassifier.logScore(phrase, label);
      const bundleProb = Math.exp(bundleLog);

      const delta = Math.abs(bundleProb - upstreamProb);
      fixtures.push({ phrase, label, upstream_probability: upstreamProb, bundle_log_score: bundleLog, delta });
      if (delta > maxDelta) {
        maxDelta = delta;
      }

      assert.ok(
        delta <= 1e-9,
        `Score parity delta ${delta} exceeds 1e-9 for phrase "${phrase}", label "${label}"`
      );
    }
  }
  log(`     Max observed probability delta: ${maxDelta.toExponential(4)} (well within 1e-9)`);

  // 8. Test OOV (Out-of-vocabulary) and Long Inputs
  log('  7. Testing OOV and long input stability...');
  const longOovText =
    'Quantum teleportation superluminal entanglement tachyons neutrino astrophysics ' +
    'cosmological singularity event horizon quasar pulsar magnetar interstellar ' +
    'spectroscopy exoplanet gravitational lensing dark matter antimatter annihilation ' +
    'xenon isotope relativistic plasma magnetohydrodynamics thermonuclear fusion.';

  const oovPresentLog = bundleClassifier.logScore(longOovText, 'present');
  const oovAbsentLog = bundleClassifier.logScore(longOovText, 'absent');

  assert.ok(Number.isFinite(oovPresentLog), 'OOV present log score must be finite');
  assert.ok(Number.isFinite(oovAbsentLog), 'OOV absent log score must be finite');
  assert.ok(!Number.isNaN(oovPresentLog), 'OOV present log score must not be NaN');
  assert.ok(!Number.isNaN(oovAbsentLog), 'OOV absent log score must not be NaN');

  // 9. Test Save / Restore Parity
  log('  8. Testing save/restore model structure and score parity...');
  const serialized = JSON.stringify(bundleClassifier);
  const restored = BundleBayes.restore(JSON.parse(serialized), BundleStemmer);
  assert.deepStrictEqual(
    restored.featureOrder,
    bundleClassifier.featureOrder,
    'Restored featureOrder must match exactly'
  );

  for (const phrase of testPhrases) {
    for (const label of ['present', 'absent']) {
      const origScore = bundleClassifier.logScore(phrase, label);
      const restScore = restored.logScore(phrase, label);
      assert.strictEqual(
        origScore,
        restScore,
        `Restored model logScore differs for "${phrase}" (${label})`
      );
    }
  }

  if (jsonMode) process.stdout.write(JSON.stringify({ tolerance: 1e-9, max_delta: maxDelta, training_corpus: trainingCorpus, fixtures }, null, 2) + '\n');
  log('All classifier bundle parity tests PASSED successfully.');
}

main().catch((err) => {
  console.error('Classifier bundle parity tests FAILED:', err);
  process.exit(1);
});
