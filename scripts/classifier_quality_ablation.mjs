#!/usr/bin/env node
/** Offline score extraction for fixed-corpus ablations. Rust calibration and
 * selection evaluate these files; this script never reads Test feature text.
 * Run on an idle host, then use classifier_quality_benchmark's opt-in evaluator.
 */
import fs from 'node:fs';
import path from 'node:path';
import vm from 'node:vm';
import crypto from 'node:crypto';
import {execFileSync} from 'node:child_process';
import {fileURLToPath} from 'node:url';
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..');
const output=path.resolve(root,process.argv[2]||'.agent-scratch/classifier-quality-v2/ablations');
if(!output.startsWith(path.join(root,'.agent-scratch')+path.sep))throw new Error('Benchmark output belongs in .agent-scratch');
fs.mkdirSync(output,{recursive:true});
const sha=bytes=>crypto.createHash('sha256').update(bytes).digest('hex');
const dataset=path.join(root,'datasets/argos_atlas_strategic_domains_v2.csv');
const selected=JSON.parse(execFileSync('python3',['-c',`import csv,json,sys
with open(sys.argv[1],newline='') as f:
 rows=[{k:r[k] for k in ['sample_id','scenario_group','split','title','description','domain_labels']} for r in csv.DictReader(f) if r['split']!='test']
print(json.dumps(rows))`,dataset],{maxBuffer:16*1024*1024}));
const baseline=execFileSync('git',['rev-parse','HEAD'],{cwd:root,encoding:'utf8'}).trim();
const source=file=>fs.readFileSync(path.join(root,file),'utf8');
const frozen=file=>execFileSync('git',['show',`${baseline}:${file}`],{cwd:root,encoding:'utf8',maxBuffer:16*1024*1024});
const base='crates/argos-osint-core/';
const domains=['governance','security','economic','human','environmental','scientific'];
const variants=[{name:'old_bayes',old:true},{name:'corrected_unigrams',phrases:false},{name:'plus_bigrams',phrases:true},{name:'plus_vocabulary_support',phrases:true,minDf:2},{name:'plus_field_features',phrases:true,minDf:2,fields:true}];
const manifest={schema_version:'classifier-ablation-scores-v2',dataset_sha256:sha(fs.readFileSync(dataset)),baseline_revision:baseline,test_opened:false,variants:[],environment:{node:process.version,platform:process.platform,arch:process.arch}};
for(const variant of variants){
 const sandbox=vm.createContext({console:{log(){},warn(){},error(){}}});
 const bundle=(variant.old?frozen:source)(base+'assets/classifier/natural-classifier.js');
 const bridge=(variant.old?frozen:source)(base+'src/classifier/bridge.js');
 const start=performance.now();
 vm.runInContext(bundle,sandbox);
 let features='',vocabulary='';
 if(!variant.old){
   features=source(base+'src/classifier/features.js');vm.runInContext(features,sandbox);
   vocabulary=source(base+'src/classifier/vocabulary.js');vm.runInContext(vocabulary,sandbox);
 }
 vm.runInContext(bridge,sandbox);
 const coldLoadMs=performance.now()-start;
 const call=(op,body)=>{
   const reply=JSON.parse(sandbox.argosClassifier[op](JSON.stringify({version:'1',request_id:`${variant.name}-${op}`,model_id:variant.name,...body})));
   if(reply.error || reply.errors?.length)throw new Error(`${variant.name}/${op}: ${JSON.stringify(reply)}`);return reply;
 };
 const train=selected.filter(row=>row.split==='train');
 const trainStart=performance.now();
 const rows=train.map(row=>({row_id:row.sample_id,feature_input_hash:sha(JSON.stringify([row.title,row.description])),title:row.title,description:row.description,membership:domains.map(domain=>row.domain_labels.split('|').includes(domain)?'Present':'Absent')}));
 if(!variant.old){
   for(let index=0;index<rows.length;index+=32)call('trainBatch',{phase:'vocabulary',feature_policy:{include_bigrams:variant.phrases,min_document_frequency:variant.minDf||1,field_namespaces:variant.fields===true},rows:rows.slice(index,index+32)});
   call('trainBatch',{phase:'freeze_vocabulary',rows:[]});
 }

 for(let index=0;index<train.length;index+=32){
   call('trainBatch',{rows:rows.slice(index,index+32)});
   while(!call('finishTraining',{max_documents_per_step:16}).done){}
 }
 const fitMs=performance.now()-trainStart;
 const payload=JSON.stringify(call('serializeModel',{}));
 const scores=[];
 for(const split of ['calibration','development']){
   const rows=selected.filter(row=>row.split===split);
   for(let index=0;index<rows.length;index+=64){
     const batch=rows.slice(index,index+64);
     const reply=call('classifyBatch',{items:batch.map(row=>({id:row.sample_id,title:row.title,description:row.description}))});
     reply.items.forEach((item,index)=>scores.push({family_id:batch[index].scenario_group,representative_id:batch[index].sample_id,split,membership:domains.map(domain=>batch[index].domain_labels.split('|').includes(domain)?'Present':'Absent'),raw:item.results.map(({domain,present_log_score,absent_log_score,raw_log_odds,known_feature_count,oov_count})=>({domain,present_log_score,absent_log_score,raw_log_odds,known_feature_count,oov_count}))}));
   }
 }
 const items=selected.filter(row=>row.split==='development').slice(0,64).map(row=>({id:row.sample_id,title:row.title,description:row.description}));
 const samples=[];for(let iteration=0;iteration<21;iteration++){const start=performance.now();call('classifyBatch',{items});samples.push(performance.now()-start);}
 const first=samples.shift();samples.sort((a,b)=>a-b);
 const encoded=JSON.stringify({schema_version:manifest.schema_version,dataset_sha256:manifest.dataset_sha256,variant:variant.name,rows:scores},null,2)+'\n';
 const file=`${variant.name}.json`;fs.writeFileSync(path.join(output,file),encoded);
 manifest.variants.push({name:variant.name,file,sha256:sha(encoded),bundle_sha256:sha(bundle),bridge_sha256:sha(bridge),features_sha256:sha(features),vocabulary_sha256:sha(vocabulary),timing_runtime:"Node V8; offline diagnostic, not shipped QuickJS",model_payload_sha256:sha(payload),calibration_families:230,development_families:165,cold_load_ms:coldLoadMs,fit_ms:fitMs,first_batch64_ms:first,warm_batch64_p50_ms:samples[9],warm_batch64_p95_ms:samples[18],rss_peak_bytes:null,rss_reason:'Use an external peak RSS sampler; process RSS after each variant does not establish a peak.'});
 process.stderr.write(`Scored ${variant.name}\n`);
}
fs.writeFileSync(path.join(output,'manifest.json'),JSON.stringify(manifest,null,2)+'\n');
process.stdout.write(path.join(output,'manifest.json')+'\n');
