/** Train-only bounded vocabulary statistics. No held-out or live article can
 * update these values. Natural TF-IDF is diagnostic/selection data; Bayes still
 * consumes binary feature presence and exact learned smoothing/counts.
 */
'use strict';
(() => {
  const VERSION='natural-en-phrases-v2';
  const MAX_TERMS=100000;
  function extract(policy,title,description) {
    return globalThis.argosClassifierFeatures.extract(title,description)
      .filter(feature=>policy.include_bigrams || !feature.phrase)
      .map(feature=>({...feature,id:policy.field_namespaces?`${feature.field}:${feature.id}`:feature.id}));
  }
  function policyOf(options={}) {
    const policy={version:VERSION,min_document_frequency:options.min_document_frequency??1,max_vocabulary:options.max_vocabulary??20000,include_bigrams:options.include_bigrams!==false,field_namespaces:options.field_namespaces===true};
    if(!Number.isInteger(policy.min_document_frequency)||policy.min_document_frequency<1||policy.min_document_frequency>50000||!Number.isInteger(policy.max_vocabulary)||policy.max_vocabulary<1||policy.max_vocabulary>MAX_TERMS)throw new Error('vocabulary_policy_bound');
    return policy;
  }
  function collect(ctx,rows,options) {
    if(ctx.classifiers.some(model=>model.docs.length || model.classifier.totalExamples>1))throw new Error('vocabulary_prepare_after_fitting');
    const requested=policyOf(options);
    let state=ctx.vocabulary;
    if(!state){state=ctx.vocabulary={policy:requested,df:new Map(),classDf:Array.from({length:6},()=>[new Map(),new Map()]),classTotals:Array.from({length:6},()=>[0,0]),documents:0,tokenBytes:0,tfidf:new globalThis.argosNaturalBayes.TfIdf(),prepared:new Map(),frozen:false};}
    if(state.frozen || JSON.stringify(state.policy)!==JSON.stringify(requested))throw new Error('vocabulary_policy_conflict');
    for(const row of rows){
      if(row.membership.every(value=>value==='Unknown'))continue;
      if(typeof row.feature_input_hash!=='string'||!/^[a-f0-9]{64}$/.test(row.feature_input_hash))throw new Error('vocabulary_row_hash');
      if(state.prepared.has(row.row_id)){if(state.prepared.get(row.row_id).hash!==row.feature_input_hash)throw new Error('vocabulary_row_conflict');continue;}
      const prepared=extract(state.policy,row.title,row.description);
      state.prepared.set(row.row_id,{hash:row.feature_input_hash,features:prepared});
      const tokens=prepared.map(feature=>feature.id);
      state.documents++;
      if(state.documents>50000)throw new Error('vocabulary_document_bound');
      state.tfidf.addDocument(tokens);
      for(const token of tokens){
        state.tokenBytes+=globalThis.argosClassifierFeatures.byteLength(token);
        if(state.tokenBytes>32*1024*1024)throw new Error('vocabulary_byte_bound');
        state.df.set(token,(state.df.get(token)||0)+1);
        if(state.df.size>MAX_TERMS)throw new Error('vocabulary_term_bound');
      }
      for(let domain=0;domain<6;domain++){
        const membership=row.membership[domain];if(membership==='Unknown')continue;
        const label=membership==='Present'?0:1;state.classTotals[domain][label]++;
        for(const token of tokens){const map=state.classDf[domain][label];map.set(token,(map.get(token)||0)+1);}
      }
    }
    return {prepared:rows.length,documents:state.documents,terms:state.df.size};
  }
  function freeze(ctx){
    const state=ctx.vocabulary;if(!state || state.frozen || !state.documents)throw new Error('vocabulary_freeze_state');
    const terms=[];
    for(const [id,df]of state.df){
      if(df<state.policy.min_document_frequency)continue;
      let association=0;
      for(let domain=0;domain<6;domain++){
        const [positive,negative]=state.classTotals[domain];if(!positive || !negative)continue;
        // Same Natural binary-presence count/smoothing convention as scoring.
        const p=((state.classDf[domain][0].get(id)||0)+1)/(positive+1);
        const n=((state.classDf[domain][1].get(id)||0)+1)/(negative+1);
        association=Math.max(association,Math.abs(Math.log(p)-Math.log(n)));
      }
      if(association<=1e-12)continue; // High IDF alone is not domain support.
      const idf=1+Math.log(state.documents/(1+df));
      terms.push({id,document_frequency:df,idf,association});
    }
    terms.sort((a,b)=>(b.association*b.idf)-(a.association*a.idf)||(a.id<b.id?-1:a.id>b.id?1:0));
    terms.length=Math.min(terms.length,state.policy.max_vocabulary);
    if(!terms.length)throw new Error('no_discriminative_training_vocabulary');
    // Check the compact DF formula against unmodified Natural on a bounded
    // diagnostic set, rather than doing an all-documents scan per term/input.
    for(const term of terms.slice(0,8))if(Math.abs(state.tfidf.idf(term.id)-term.idf)>1e-12)throw new Error('tfidf_statistics_mismatch');
    state.manifest={...state.policy,training_documents:state.documents,terms};
    state.allowed=new Set(terms.map(term=>term.id));state.frozen=true;
    delete state.df;delete state.classDf;delete state.classTotals;delete state.tfidf;
    return state.manifest;
  }
  function features(ctx,title,description){
    const state=ctx.vocabulary;
    const result=extract(state?.policy||policyOf(),title,description);
    return result;
  }
  function trainingFeatures(ctx,row){
    const state=ctx.vocabulary;
    if(!state?.frozen)return features(ctx,row.title,row.description);
    const prepared=state.prepared?.get(row.row_id);
    if(!prepared || prepared.hash!==row.feature_input_hash)throw new Error('unprepared_training_row');
    state.prepared.delete(row.row_id);
    return prepared.features.filter(feature=>state.allowed.has(feature.id));
  }
  function restore(ctx,manifest){
    if(!manifest)return;
    const policy=policyOf(manifest);
    if(manifest.version!==VERSION || !Number.isInteger(manifest.training_documents)||manifest.training_documents<1||manifest.training_documents>50000||!Array.isArray(manifest.terms)||manifest.terms.length<1||manifest.terms.length>policy.max_vocabulary)throw new Error('invalid_vocabulary_manifest');
    const seen=new Set();
    for(const term of manifest.terms){if(!term || typeof term.id!=='string'||seen.has(term.id)||!Number.isInteger(term.document_frequency)||term.document_frequency<policy.min_document_frequency||term.document_frequency>manifest.training_documents||!Number.isFinite(term.idf)||Math.abs(term.idf-(1+Math.log(manifest.training_documents/(1+term.document_frequency))))>1e-12||!Number.isFinite(term.association)||term.association<=0)throw new Error('invalid_vocabulary_term');seen.add(term.id);}
    ctx.vocabulary={policy,manifest,allowed:seen,frozen:true};
  }
  globalThis.argosClassifierVocabulary=Object.freeze({version:VERSION,collect,freeze,features,trainingFeatures,restore});
})();
