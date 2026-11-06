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

  for(const [left,right] of [['Assembly approves budget','Assembly approves budget'],['Hospital will open on Friday','Hospital will not open on Friday'],['Reported 12 cases','Reported 21 cases'],['A','B'],['Council monitors flood','council  monitors flood']]) {
    assert.equal(sandbox.argosNaturalBayes.DiceCoefficient(left,right),natural.DiceCoefficient(left,right));
    assert.equal(sandbox.argosNaturalBayes.JaroWinklerDistance(left,right,{ignoreCase:true}),natural.JaroWinklerDistance(left,right,{ignoreCase:true}));
  }

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
  assert.deepStrictEqual(bundleClassifier.classifier.classFeatures,upstreamClassifier.classifier.classFeatures,'Sparse fitting must preserve upstream binary feature counts');
  assert.deepStrictEqual(bundleClassifier.classifier.classTotals,upstreamClassifier.classifier.classTotals);
  assert.equal(bundleClassifier.classifier.totalExamples,upstreamClassifier.classifier.totalExamples);

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

  log('  9. Testing phrase boundaries, frozen features and sparse score parity...');
  new Function('globalThis', fs.readFileSync(path.join(REPO_ROOT,'crates/argos-osint-core/src/classifier/features.js'),'utf8'))(sandbox);
  const features=sandbox.argosClassifierFeatures;
  const extract=(title,description='')=>features.extract(title,description);
  for (const text of ['This was the','are the','was in','the and of','those are the','it was in the']) assert.deepStrictEqual(extract(text),[],`Neutral-only phrases cannot become support: ${text}`);
  assert.ok(extract('are not').some(feature=>feature.id==='u:"not"'),'Noise filtering must preserve negation');
  for (const [title,description] of [['armed','strike'],['armed. Strike',''],['armed, strike',''],['armed; strike',''],['armed\nstrike',''],['armed\r\nstrike',''],['armed\u2029strike','']]) {
    assert.ok(!extract(title,description).some(feature=>feature.id==='b:["arm","strike"]'), 'No phrase may cross a field or punctuation boundary');
  }
  assert.ok(extract('armed strike').some(feature=>feature.id==='b:["arm","strike"]'));
  assert.ok(extract('the and of').length===0, 'Function words alone cannot establish support');
  const special=extract("NATO denies CVE-2026-1234 isn't resolved; U.S. teams re-test 42 systems");
  for (const id of ['u:"nato"','u:"cve-2026-1234"','u:"not"','u:"u.s."','u:"42"']) assert.ok(special.some(feature=>feature.id===id), id);
  const unicode='é NATO';
  const span=extract(unicode).find(feature=>feature.id==='u:"nato"');
  assert.equal(span.start_byte,Buffer.byteLength('é '));
  assert.equal(span.end_byte,Buffer.byteLength(unicode));
  assert.deepStrictEqual(sandbox.argosNaturalBayes.NGrams.bigrams(['a','b','c']),natural.NGrams.bigrams(['a','b','c']));
  const frozen=new sandbox.argosNaturalBayes.TfIdf();
  assert.equal(frozen.addFileSync,undefined,'Runtime must not expose filesystem entry points');
  const oracle=new natural.TfIdf();
  for (const tokens of [['u:"ballot"','b:["arm","strike"]'],['u:"ballot"','u:"poll"']]) {frozen.addDocument(tokens);oracle.addDocument(tokens);}
  for (const token of ['u:"ballot"','u:"poll"']) assert.equal(frozen.idf(token),oracle.idf(token));
  for (const phrase of testPhrases) {
    const tokens=BundleStemmer.tokenizeAndStem(phrase,true);
    const ids=bundleClassifier.featureOrder.map((feature,index)=>tokens.includes(feature)?index:-1).filter(index=>index>=0);
    const sparse=bundleClassifier.logScoresActive(ids);
    for (const [index,label] of ['present','absent'].entries()) assert.equal(sparse[index],bundleClassifier.logScore(tokens,label), 'Sparse scoring must preserve exact summation order');
  }

  const incrementalOracle=new natural.BayesClassifier(natural.PorterStemmer);
  const incrementalBundle=new BundleBayes(BundleStemmer);
  for(let round=0;round<4;round++){
    for(let index=0;index<17;index++){
      const tokens=Array.from({length:11},(_,j)=>`u:${JSON.stringify(`term${(index*7+j*3+round*13)%53}`)}`);tokens.push(tokens[0]);
      const label=index%2?'present':'absent';incrementalOracle.addDocument(tokens,label);incrementalBundle.addDocument(tokens,label);
    }
    incrementalOracle.train();while(!incrementalBundle.trainStep(3)){}
    for(const key of ['classFeatures','classTotals','totalExamples','smoothing'])assert.deepStrictEqual(incrementalBundle.classifier[key],incrementalOracle.classifier[key],`Incremental sparse fit ${key}`);
    for(let index=0;index<53;index++)for(const label of ['present','absent']){
      const tokens=[`u:"term${index}"`,'u:"unseen"'];
      const expected=incrementalOracle.classifier.probabilityOfClass(incrementalOracle.textToFeatures(tokens),label);
      assert.ok(Math.abs(incrementalBundle.logScore(tokens,label)-Math.log(expected))<1e-9);
    }
  }

  log('  10. Testing frozen Train vocabulary and held-out isolation...');
  new Function('globalThis',fs.readFileSync(path.join(REPO_ROOT,'crates/argos-osint-core/src/classifier/vocabulary.js'),'utf8'))(sandbox);
  new Function('globalThis',fs.readFileSync(path.join(REPO_ROOT,'crates/argos-osint-core/src/classifier/bridge.js'),'utf8'))(sandbox);
  let sequence=0;
  const bridge=(operation,body)=>{
    const reply=JSON.parse(sandbox.argosClassifier[operation](JSON.stringify({version:'1',request_id:`vocabulary-${sequence++}`,model_id:'frozen-vocabulary',...body})));
    assert.ok(!reply.error,JSON.stringify(reply));
    return reply;
  };
  const rows=[{row_id:'positive',feature_input_hash:'a'.repeat(64),title:'Ballot parliamentary vote',description:'Lawmakers ratify legislation',membership:Array(6).fill('Present')},{row_id:'negative',feature_input_hash:'b'.repeat(64),title:'Culinary recipe feast',description:'Bakers prepare pastry',membership:Array(6).fill('Absent')}];
  assert.equal(bridge('trainBatch',{phase:'vocabulary',rows}).prepared,2);
  assert.equal(bridge('trainBatch',{phase:'freeze_vocabulary',rows:[]}).vocabulary_frozen,true);
  bridge('trainBatch',{rows});
  while(!bridge('finishTraining',{max_documents_per_step:16}).done){}
  const original=bridge('serializeModel',{});
  assert.equal(original.feature_policy.training_documents,2);
  assert.ok(original.feature_policy.terms.every(term=>term.document_frequency>=1 && Number.isFinite(term.idf)));
  const heldout={items:[{id:'held-out',title:'quasar unseen vocabulary',description:'unknown astronomical discovery'}]};
  const before=JSON.stringify(original.feature_policy);
  const unseen=bridge('classifyBatch',heldout);
  assert.ok(unseen.items[0].results.every(result=>result.known_feature_count===0));
  assert.equal(JSON.stringify(bridge('serializeModel',{}).feature_policy),before,'Held-out inference cannot update frozen DF/IDF or vocabulary');
  assert.throws(()=>sandbox.argosClassifierVocabulary.restore({}, {...original.feature_policy,terms:[{...original.feature_policy.terms[0],idf:123}]}),/invalid_vocabulary_term/);

  let bindingReads=0;
  class AuditedBayes extends BundleBayes {
    constructor(...args){super(...args);this.featureIndex=new Proxy(this.featureIndex,{get(target,key,receiver){if(typeof key==='string' && /^\d+$/.test(key))bindingReads++;return Reflect.get(target,key,receiver);}});}
  }
  sandbox.argosNaturalBayes={...sandbox.argosNaturalBayes,BayesClassifier:AuditedBayes};
  new Function('globalThis',fs.readFileSync(path.join(REPO_ROOT,'crates/argos-osint-core/src/classifier/bridge.js'),'utf8'))(sandbox);
  const auditCall=(operation,body)=>{const value=JSON.parse(sandbox.argosClassifier[operation](JSON.stringify({version:'1',request_id:`audit-${sequence++}`,model_id:'batch-audit',...body})));assert.ok(!value.error,JSON.stringify(value));return value;};
  const auditRows=rows.map(({feature_input_hash,...row})=>row);
  auditCall('trainBatch',{rows:auditRows});while(!auditCall('finishTraining',{max_documents_per_step:16}).done){}
  const audited=auditCall('serializeModel',{});const totalVocabulary=Object.values(audited.domains).reduce((sum,model)=>sum+model.feature_order.length,0);
  bindingReads=0;
  auditCall('trainBatch',{rows:Array.from({length:32},(_,index)=>({...auditRows[index%2],row_id:`batch-row-${index}`}))});
  assert.ok(bindingReads<=3*totalVocabulary,`Binding audits must scale with vocabulary per batch, not per row: ${bindingReads}/${totalVocabulary}`);

  if (jsonMode) process.stdout.write(JSON.stringify({ tolerance: 1e-9, max_delta: maxDelta, training_corpus: trainingCorpus, fixtures }, null, 2) + '\n');
  log('All classifier bundle parity tests PASSED successfully.');
}

main().catch((err) => {
  console.error('Classifier bundle parity tests FAILED:', err);
  process.exit(1);
});
