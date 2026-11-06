#!/usr/bin/env node
// Offline, independently generated Natural oracle. Rust never invokes Node.
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import {fileURLToPath} from 'node:url';
import {createRequire} from 'node:module';
import assert from 'node:assert/strict';
const root = path.dirname(fileURLToPath(import.meta.url));
const repo = path.resolve(root, '../../../../..');
const require = createRequire(path.join(repo, 'package.json'));
const natural = require('natural');
assert.equal(require('natural/package.json').version, '8.2.6');
assert.equal(require('apparatus/package.json').version, '0.0.10');
const sha = bytes => crypto.createHash('sha256').update(bytes).digest('hex');
const recipeBytes = fs.readFileSync(path.join(root, 'six-model.json'));
const recipe = JSON.parse(recipeBytes);
const assets = path.join(repo, 'crates/argos-osint-core/assets/classifier');
const manifestBytes = fs.readFileSync(path.join(assets, 'source-manifest.json'));
const manifest = JSON.parse(manifestBytes);
const domains = ['governance','security','economic','human','environmental','scientific'];
const models = domains.map((domain, index) => {
  const classifier = new natural.BayesClassifier(natural.PorterStemmer);
  classifier.setOptions({keepStops:true});
  const featureOrder = [];
  for (const row of recipe.rows) {
    const label = row.membership[index];
    if (label === 'Unknown') continue;
    const tokens = natural.PorterStemmer.tokenizeAndStem(`${row.title} ${row.description}`.trim(), true);
    for (const token of tokens) if (!featureOrder.includes(token)) featureOrder.push(token);
    classifier.addDocument(tokens, label.toLowerCase());
  }
  classifier.train();
  const queries = recipe.queries.map(text => {
    const tokens = natural.PorterStemmer.tokenizeAndStem(text, true);
    const features = classifier.textToFeatures(tokens);
    const present = Math.log(classifier.classifier.probabilityOfClass(features, 'present'));
    const absent = Math.log(classifier.classifier.probabilityOfClass(features, 'absent'));
    assert(Number.isFinite(present) && Number.isFinite(absent));
    return {text, tokens, binary_features: featureOrder.map(token => tokens.includes(token) ? 1 : 0),
      present_log_score:present, absent_log_score:absent, raw_log_odds:present-absent};
  });
  return {domain, feature_order:featureOrder, labels:['present','absent'], queries};
});
const oracle = {schema_version:'classifier-upstream-oracle-v1', natural_version:'8.2.6',apparatus_version:'0.0.10',
  tolerance:1e-9, recipe_sha256:sha(recipeBytes),source_manifest_sha256:sha(manifestBytes),
  bundle_sha256:manifest.build.bundle_sha256,adapter_sha256:manifest.adaptation.adapter_sha256,
  bridge_sha256:manifest.bridge.sha256,models};
const output = JSON.stringify(oracle,null,2)+'\n';
const dest=path.join(root,'bayes-parity.json');
if (process.argv.includes('--write')) fs.writeFileSync(dest, output);
else if (process.argv.includes('--check')) assert.equal(fs.readFileSync(dest,'utf8'),output,'upstream oracle drift; review before --write');
else process.stdout.write(output);
