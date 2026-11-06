#!/usr/bin/env node
/** Generate the six-model QuickJS oracle using unmodified pinned upstream
 * Natural probabilities and the independently tested shared feature policy.
 * Run bundle parity first. This does not evaluate corpus quality.
 */
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import vm from 'node:vm';
import {fileURLToPath} from 'node:url';
import natural from 'natural';
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..');
const hash=data=>crypto.createHash('sha256').update(data).digest('hex');
const dir=path.join(root,'crates/argos-osint-core/tests/fixtures/classifier');
const recipeBytes=fs.readFileSync(path.join(dir,'six-model.json'));
const recipe=JSON.parse(recipeBytes);
const manifestPath=path.join(root,'crates/argos-osint-core/assets/classifier/source-manifest.json');
const manifest=JSON.parse(fs.readFileSync(manifestPath));
const sandbox=vm.createContext({argosNaturalBayes:{PorterStemmer:natural.PorterStemmer,NGrams:natural.NGrams,RegexpTokenizer:natural.RegexpTokenizer}});
vm.runInContext(fs.readFileSync(path.join(root,'crates/argos-osint-core/src/classifier/features.js'),'utf8'),sandbox);
const tokens=(title,description='')=>Array.from(sandbox.argosClassifierFeatures.extract(title,description),feature=>feature.id);
const domains=['governance','security','economic','human','environmental','scientific'];
const queries=['ballot patrol common','cannot never NATO 2026','freight hospital common','river laboratory common','zzqxjv',''];
const result={schema_version:'classifier-upstream-oracle-v2',natural_version:'8.2.6',apparatus_version:'0.0.10',tolerance:1e-9,recipe_sha256:hash(recipeBytes),source_manifest_sha256:hash(fs.readFileSync(manifestPath)),bundle_sha256:manifest.build.bundle_sha256,adapter_sha256:manifest.adaptation.adapter_sha256,bridge_sha256:manifest.bridge.sha256,features_sha256:manifest.features.sha256,models:[]};
for(const [index,domain]of domains.entries()){
 const model=new natural.BayesClassifier(natural.PorterStemmer,1);model.setOptions({keepStops:true});
 for(const row of recipe.rows)if(row.membership[index]!=='Unknown')model.addDocument(tokens(row.title,row.description),row.membership[index]==='Present'?'present':'absent');
 model.train();
 result.models.push({domain,feature_order:Object.keys(model.features),queries:queries.map(text=>{
   const observation=model.textToFeatures(tokens(text));
   // The oracle uses upstream probabilityOfClass without the adapted log scorer.
   const present=Math.log(model.classifier.probabilityOfClass(observation,'present'));
   const absent=Math.log(model.classifier.probabilityOfClass(observation,'absent'));
   if(!Number.isFinite(present)||!Number.isFinite(absent))throw new Error('Oracle fixture underflow');
   return {text,present_log_score:present,absent_log_score:absent,raw_log_odds:present-absent};
 })});
}
const file=path.join(dir,'bayes-parity.json');
fs.writeFileSync(file,JSON.stringify(result,null,2)+'\n');process.stdout.write(file+'\n');
