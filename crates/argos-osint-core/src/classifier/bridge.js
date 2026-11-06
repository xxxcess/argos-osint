/**
 * Argos Six-Domain Classifier JavaScript Bridge
 *
 * Implements globalThis.argosClassifier with five JSON-in / JSON-out operations:
 * - trainBatch
 * - finishTraining
 * - classifyBatch
 * - serializeModel
 * - restoreModel
 *
 * Owns Argos policies:
 * - Exactly six canonical domains in DomainId::ALL order:
 *   [governance, security, economic, human, environmental, scientific]
 * - Masked partial labels (Unknown membership omitted from binary model)
 * - Single tokenization pass per document row
 * - Log-space scoring log(P_present) - log(P_absent) without absent-token Bernoulli factor
 * - Input batch limit of 64 items
 * - Prototype pollution protection (__proto__, prototype, constructor)
 */

'use strict';

(() => {
  const DOMAIN_IDS = [
    'governance',
    'security',
    'economic',
    'human',
    'environmental',
    'scientific',
  ];

  const BRIDGE_VERSION = 'argos-classifier-bridge-v2';
  const PAYLOAD_SCHEMA_VERSION = 'classifier-payload-v1';
  const TAXONOMY_VERSION = 'strategic-domains-v1';
  const FEATURE_SCHEMA_VERSION = 'classifier-features-v3';
  const ALGORITHM_VERSION = 'natural-bayes-binary-ovr-v1';
  const PREPROCESSING_VERSION = 'natural-en-phrases-v2';

  // In-memory model registry: model_id -> ModelContext
  const models = new Map();

  function assertNoPollution(obj) {
    if (obj && typeof obj === 'object') {
      for (const key of Object.keys(obj)) {
        if (
          key === '__proto__' ||
          key === 'prototype' ||
          key === 'constructor'
        ) {
          throw new Error('prototype pollution key rejected: ' + key);
        }
        assertNoPollution(obj[key]);
      }
    }
  }

  function getOrCreateContext(modelId) {
    let ctx = models.get(modelId);
    if (!ctx) {
      const { BayesClassifier, PorterStemmer } = globalThis.argosNaturalBayes;
      const classifiers = DOMAIN_IDS.map(() => {
        const c = new BayesClassifier(PorterStemmer, 1.0);
        c.setOptions({ keepStops: true });
        return c;
      });
      ctx = {
        modelId,
        classifiers,
        stemmer: PorterStemmer,
        featureOrderRevisions: [0, 0, 0, 0, 0, 0],
        round: 0,
        fitStep: 0,
      };
      models.set(modelId, ctx);
    }
    return ctx;
  }

  function requireFeatureOrder(classifier, domain) {
    if (!Array.isArray(classifier.featureOrder) || !Array.isArray(classifier.featureIndex) || classifier.featureIndex.length !== classifier.featureOrder.length || classifier.featureIndex.some((binding, index) => !binding || binding.index !== index || binding.token !== classifier.featureOrder[index])) throw new Error('feature_order_mismatch: ' + domain);
    return classifier.featureOrder;
  }

  function pendingFit(ctx) { return ctx.classifiers.some(c => c.lastAdded < c.docs.length); }
  function utf8Bytes(text) {
    let bytes = 0;
    for (const character of text) { const code = character.codePointAt(0); bytes += code < 128 ? 1 : code < 2048 ? 2 : code < 65536 ? 3 : 4; }
    return bytes;
  }
  function validIdentifier(value) { return typeof value === 'string' && value.length > 0 && utf8Bytes(value) <= 256 && !/[\u0000-\u001f\u007f-\u009f]/.test(value); }

  function trainBatch(requestJson) {
    try {
      const req = JSON.parse(requestJson);
      assertNoPollution(req);

      const modelId = req.model_id || 'default';
      const ctx = getOrCreateContext(modelId);
      if (pendingFit(ctx)) throw new Error('training_round_pending');
      if (!Array.isArray(req.rows) || req.rows.length > 32) throw new Error('invalid_training_batch_size');
      const rows = req.rows;
      for (const row of rows) {
        if (!row || !validIdentifier(row.row_id) || typeof row.title !== 'string' || typeof row.description !== 'string' || utf8Bytes(row.title) + utf8Bytes(row.description) > 32768 || !Array.isArray(row.membership) || row.membership.length !== 6 || row.membership.some(value => !['Present','Absent','Unknown'].includes(value))) throw new Error('invalid_training_row');
      }
      if(req.phase==='vocabulary') {
        const prepared=globalThis.argosClassifierVocabulary.collect(ctx,rows,req.feature_policy);
        return JSON.stringify({version:'1',request_id:req.request_id,...prepared,errors:[]});
      }
      if(req.phase==='freeze_vocabulary') {
        const manifest=globalThis.argosClassifierVocabulary.freeze(ctx);
        return JSON.stringify({version:'1',request_id:req.request_id,vocabulary_frozen:true,terms:manifest.terms.length,documents:manifest.training_documents,errors:[]});
      }
      for (let domain = 0; domain < 6; domain++) {
        const added = rows.filter(row => row.membership[domain] !== 'Unknown').length;
        if (ctx.classifiers[domain].classifier.totalExamples - 1 + ctx.classifiers[domain].docs.length - ctx.classifiers[domain].lastAdded + added > 50000) throw new Error('training_document_bound');
      }
      ctx.round++;

      let accepted = 0;
      let skipped = 0;
      let tokenBytes = 0;
      let vocabNew = 0;

      // Only compiled addDocument can mutate this append-only order. Audit the
      // existing prefix once per batch, then audit the complete result once.
      ctx.classifiers.forEach((classifier,index)=>requireFeatureOrder(classifier,DOMAIN_IDS[index]));
      for (const row of rows) {
        if (!row || typeof row !== 'object') {
          skipped++;
          continue;
        }

        const text = ((row.title || '') + ' ' + (row.description || '')).trim();
        if (text.length === 0) {
          skipped++;
          continue;
        }

        // Tokenize text exactly once for all 6 domains
        const tokens = globalThis.argosClassifierVocabulary.trainingFeatures(ctx,row).map(feature => feature.id);
        if (tokens.length === 0) {
          skipped++;
          continue;
        }

        for (const t of tokens) {
          tokenBytes += utf8Bytes(t);
        }

        const membership = Array.isArray(row.membership) ? row.membership : [];
        let rowContributed = false;

        for (let d = 0; d < 6; d++) {
          const mem = membership[d];
          const classifier = ctx.classifiers[d];
          const prevOrderLen = classifier.featureOrder.length;

          if (mem === 'Present') {
            classifier.addDocument(tokens, 'present');
            rowContributed = true;
          } else if (mem === 'Absent') {
            classifier.addDocument(tokens, 'absent');
            rowContributed = true;
          }
          // If 'Unknown', omitted entirely (masked partial label)

          const newOrderLen = classifier.featureOrder.length;
          if (newOrderLen > 100000) throw new Error('training_feature_bound');
          if (newOrderLen > prevOrderLen) {
            vocabNew += newOrderLen - prevOrderLen;
            ctx.featureOrderRevisions[d]++;
          }
        }

        if (rowContributed) {
          accepted++;
        } else {
          skipped++;
        }
      }

      const vocabSizes = ctx.classifiers.map((classifier,index)=>requireFeatureOrder(classifier,DOMAIN_IDS[index]).length);

      return JSON.stringify({
        version: req.version || '1',
        request_id: req.request_id || '',
        accepted,
        skipped,
        token_bytes: tokenBytes,
        vocabulary_new: vocabNew,
        progress: 1.0,
        round: ctx.round,
        fitted_memberships: ctx.classifiers.map(c => c.classifier.totalExamples - 1),
        vocabulary_sizes: vocabSizes,
        feature_order_revisions: ctx.featureOrderRevisions,
        errors: [],
      });
    } catch (err) {
      return JSON.stringify({
        version: '1',
        error: String(err && err.message ? err.message : err),
        code: String(err && err.message ? err.message : err).startsWith('feature_order_mismatch') ? 'feature_order_mismatch' : 'bridge_error',
        errors: [String(err && err.message ? err.message : err)],
      });
    }
  }

  function finishTraining(requestJson) {
    try {
      const req = JSON.parse(requestJson);
      assertNoPollution(req);

      const modelId = req.model_id || 'default';
      const ctx = models.get(modelId);
      if (!ctx) {
        throw new Error('model not found: ' + modelId);
      }

      const maxDocs = req.max_documents_per_step === undefined ? 16 : req.max_documents_per_step;
      if (!Number.isInteger(maxDocs) || maxDocs < 1 || maxDocs > 16) throw new Error('invalid_fit_step_budget');
      let remaining = maxDocs;
      let fitted = 0;
      for (let domain = 0; domain < 6; domain++) {
        const classifier = ctx.classifiers[domain];
        const count = Math.min(remaining, classifier.docs.length - classifier.lastAdded);
        if (count > 0) {
          classifier.trainStep(count);
          fitted += count;
          remaining -= count;
        }
      }
      if (fitted > 0) ctx.fitStep++;
      const allDone = !pendingFit(ctx);
      // No order mutation is permitted while a fitting round is pending.
      // The batch audited all bindings; trainStep validates each changed order.
      const vocabSizes = ctx.classifiers.map(classifier=>{
        if(!Array.isArray(classifier.featureOrder) || !Array.isArray(classifier.featureIndex) || classifier.featureOrder.length!==classifier.featureIndex.length)throw new Error('feature_order_mismatch');
        return classifier.featureOrder.length;
      });
      const frozenLen = vocabSizes.reduce((total, size) => total + size, 0);

      return JSON.stringify({
        version: req.version || '1',
        request_id: req.request_id || '',
        done: allDone,
        frozen_order_length: frozenLen,
        fit_step: ctx.fitStep,
        round: ctx.round,
        fitted_memberships: ctx.classifiers.map(c => c.classifier.totalExamples - 1),
        vocabulary_sizes: vocabSizes,
        feature_order_revisions: ctx.featureOrderRevisions,
        errors: [],
      });
    } catch (err) {
      return JSON.stringify({
        version: '1',
        error: String(err && err.message ? err.message : err),
        code: String(err && err.message ? err.message : err).startsWith('feature_order_mismatch') ? 'feature_order_mismatch' : 'bridge_error',
        errors: [String(err && err.message ? err.message : err)],
      });
    }
  }

  function classifyBatch(requestJson) {
    try {
      const req = JSON.parse(requestJson);
      assertNoPollution(req);

      const modelId = req.model_id || 'default';
      const ctx = models.get(modelId);
      if (!ctx) {
        throw new Error('model not found: ' + modelId);
      }
      if (pendingFit(ctx)) throw new Error('training_round_pending');

      const items = Array.isArray(req.items) ? req.items : [];
      if (items.length > 64) {
        throw new Error('batch size exceeds limit of 64 items');
      }

      const itemResults = [];

      for (const item of items) {
        const text = ((item.title || '') + ' ' + (item.description || '')).trim();
        const featureSpans = globalThis.argosClassifierVocabulary.features(ctx,item.title || '', item.description || '');
        const tokens = featureSpans.map(feature => feature.id);
        const uniqueTokens = Array.from(new Set(tokens));

        const domainScores = [];

        for (let d = 0; d < 6; d++) {
          const domain = DOMAIN_IDS[d];
          const classifier = ctx.classifiers[d];

          // Validate vocabulary once per changed order, then traverse only active IDs.
          ctx.lookups ||= [];
          if (!ctx.lookups[d] || ctx.lookups[d].size !== classifier.featureOrder.length) {
            const order = requireFeatureOrder(classifier, domain);
            ctx.lookups[d] = new Map(order.map((token,index)=>[token,index]));
          }
          const lookup = ctx.lookups[d];
          const active = tokens.filter(token=>lookup.has(token)).map(token=>lookup.get(token)).sort((a,b)=>a-b);
          const [presentLog, absentLog] = classifier.logScoresActive(active);
          const rawLogOdds = presentLog - absentLog;
          const model = classifier.classifier;
          const prior = Math.log(model.classTotals.present/model.classTotals.absent);
          const contributions = featureSpans.filter(feature=>lookup.has(feature.id)).map(feature=>{
            const index=lookup.get(feature.id);
            const positive=(model.classFeatures.present[index]||model.smoothing)/model.classTotals.present;
            const negative=(model.classFeatures.absent[index]||model.smoothing)/model.classTotals.absent;
            return {...feature, contribution:Math.log(positive)-Math.log(negative)};
          });
          const support=[];
          // A phrase and its overlapping unigram count as one support span.
          for (const feature of contributions.filter(f=>f.contribution>1e-12).sort((a,b)=>b.contribution-a.contribution)) {
            if (!support.some(other=>other.field===feature.field && other.start_byte<feature.end_byte && feature.start_byte<other.end_byte)) support.push(feature);
          }
          const knownCount=support.length;
          const oovCount=tokens.length-active.length;
          domainScores.push({
            domain,
            present_log_score: presentLog,
            absent_log_score: absentLog,
            raw_log_odds: rawLogOdds,
            known_feature_count: knownCount,
            oov_count: oovCount,
            support: {positive_spans:knownCount, negative_features:contributions.filter(f=>f.contribution< -1e-12).length, vocabulary_matches:active.length, oov_fraction:tokens.length ? oovCount/tokens.length : 1, active_phrases:contributions.filter(f=>f.phrase).length, class_prior:prior,
              positive:(item.explain ? contributions : []).filter(f=>f.contribution>0).sort((a,b)=>b.contribution-a.contribution).slice(0,8),
              negative:(item.explain ? contributions : []).filter(f=>f.contribution<0).sort((a,b)=>a.contribution-b.contribution).slice(0,8)},
          });
        }

        itemResults.push({
          id: item.id || '',
          results: domainScores,
        });
      }

      return JSON.stringify({
        version: req.version || '1',
        request_id: req.request_id || '',
        items: itemResults,
        errors: [],
      });
    } catch (err) {
      return JSON.stringify({
        version: '1',
        error: String(err && err.message ? err.message : err),
        code: String(err && err.message ? err.message : err).startsWith('feature_order_mismatch') ? 'feature_order_mismatch' : 'bridge_error',
        errors: [String(err && err.message ? err.message : err)],
      });
    }
  }

  function serializeModel(requestJson) {
    try {
      const req = JSON.parse(requestJson);
      assertNoPollution(req);

      const modelId = req.model_id || 'default';
      const ctx = models.get(modelId);
      if (!ctx) {
        throw new Error('model not found: ' + modelId);
      }
      if (pendingFit(ctx)) throw new Error('training_round_pending');

      const domainsObj = {};

      for (let d = 0; d < 6; d++) {
        const domain = DOMAIN_IDS[d];
        const classifier = ctx.classifiers[d];
        const apparatus = classifier.classifier;

        domainsObj[domain] = {
          feature_order: requireFeatureOrder(classifier, domain),
          feature_index: classifier.featureIndex,
          document_frequency: classifier.featureOrder.map(token=>classifier.features[token]),
          class_features: apparatus.classFeatures || {},
          class_totals: apparatus.classTotals || {},
          total_examples: apparatus.totalExamples || 1,
          smoothing: apparatus.smoothing !== undefined ? apparatus.smoothing : 1.0,
        };
      }

      const payload = {
        schema_version: PAYLOAD_SCHEMA_VERSION,
        feature_schema_version: FEATURE_SCHEMA_VERSION,
        bridge_version: BRIDGE_VERSION,
        taxonomy_version: TAXONOMY_VERSION,
        algorithm_version: ALGORITHM_VERSION,
        preprocessing_version: PREPROCESSING_VERSION,
        feature_policy: ctx.vocabulary?.manifest || null,
        domains: domainsObj,
      };

      return JSON.stringify(payload);
    } catch (err) {
      return JSON.stringify({
        version: '1',
        error: String(err && err.message ? err.message : err),
        code: String(err && err.message ? err.message : err).startsWith('feature_order_mismatch') ? 'feature_order_mismatch' : 'bridge_error',
        errors: [String(err && err.message ? err.message : err)],
      });
    }
  }

  function restoreModel(requestJson) {
    try {
      const req = JSON.parse(requestJson);
      assertNoPollution(req);

      let payload = req.artifact;
      if (typeof payload === 'string') {
        payload = JSON.parse(payload);
      }
      assertNoPollution(payload);

      if (!payload || typeof payload !== 'object') {
        throw new Error('invalid artifact payload: not an object');
      }

      if (payload.feature_schema_version !== FEATURE_SCHEMA_VERSION) throw new Error('incompatible_feature_schema');
      const domainsObj = payload.domains;
      if (!domainsObj || typeof domainsObj !== 'object') {
        throw new Error('payload is missing domains object');
      }

      // Exact 6 canonical domains validation
      const keys = Object.keys(domainsObj);
      if (keys.length !== 6) {
        throw new Error(
          'payload domains must contain exactly 6 keys, found ' + keys.length
        );
      }
      for (const d of DOMAIN_IDS) {
        if (!domainsObj[d] || typeof domainsObj[d] !== 'object') {
          throw new Error('missing or non-object domain: ' + d);
        }
      }

      const { BayesClassifier, PorterStemmer } = globalThis.argosNaturalBayes;
      const restoredClassifiers = [];

      for (let d = 0; d < 6; d++) {
        const domain = DOMAIN_IDS[d];
        const dData = domainsObj[domain];

        // Validate label set is exactly {"present", "absent"}
        const classTotals = dData.class_totals || {};
        const labels = Object.keys(classTotals).sort();
        if (labels.length !== 2 || labels[0] !== 'absent' || labels[1] !== 'present') {
          throw new Error(
            'domain `' + domain + '`: invalid label set, expected exactly {present, absent}'
          );
        }

        // Validate nonzero support (classTotals > 1 due to smoothing pseudocount)
        if (classTotals.present <= 1 || classTotals.absent <= 1) {
          throw new Error(
            'domain `' + domain + '`: classes must have nonzero training support'
          );
        }

        // Reconstruct classifier
        const c = new BayesClassifier(PorterStemmer, dData.smoothing || 1.0);
        c.setOptions({ keepStops: true });

        const featureOrder = dData.feature_order;
        if (!Array.isArray(featureOrder) || featureOrder.length === 0 || featureOrder.some(token => typeof token !== 'string') || new Set(featureOrder).size !== featureOrder.length) {
          throw new Error('feature_order_mismatch: ' + domain);
        }
        for (const label of ['present', 'absent']) {
          const features = dData.class_features && dData.class_features[label];
          if (!features || Object.keys(features).some(index => !/^(0|[1-9][0-9]*)$/.test(index) || Number(index) >= featureOrder.length)) {
            throw new Error('feature_order_mismatch: ' + domain);
          }
        }
        const featureIndex = dData.feature_index;
        if (!Array.isArray(featureIndex) || featureIndex.length !== featureOrder.length || featureIndex.some((binding, index) => !binding || typeof binding !== 'object' || Object.keys(binding).length !== 2 || binding.index !== index || binding.token !== featureOrder[index])) {
          throw new Error('feature_order_mismatch: ' + domain);
        }
        c.featureOrder = featureOrder;
        c.featureIndex = featureIndex;
        c.features = {};
        for (let i = 0; i < featureOrder.length; i++) {
          c.features[featureOrder[i]] = dData.document_frequency ? dData.document_frequency[i] : 1;
        }

        c.classifier.classFeatures = dData.class_features || {};
        c.classifier.classTotals = dData.class_totals || {};
        c.classifier.totalExamples = dData.total_examples || 1;
        c.classifier.smoothing = dData.smoothing !== undefined ? dData.smoothing : 1.0;

        restoredClassifiers.push(BayesClassifier.restore(c, PorterStemmer));
      }

      // Smoke inference verification on dummy input
      const smokeText = 'Smoke test classification verification';
      const smokeTokens = PorterStemmer.tokenizeAndStem(smokeText, true);
      for (const c of restoredClassifiers) {
        const presentLog = c.logScore(smokeTokens, 'present');
        const absentLog = c.logScore(smokeTokens, 'absent');
        if (!Number.isFinite(presentLog) || !Number.isFinite(absentLog)) {
          throw new Error('smoke inference produced non-finite score');
        }
      }

      const modelId = req.restored_model_id || req.model_id || 'default';
      const ctx = {
        modelId,
        classifiers: restoredClassifiers,
        stemmer: PorterStemmer,
        featureOrderRevisions: [0, 0, 0, 0, 0, 0],
        round: 0,
        fitStep: 0,
      };
      globalThis.argosClassifierVocabulary.restore(ctx,payload.feature_policy);
      models.set(modelId, ctx);

      return JSON.stringify({
        version: req.version || '1',
        request_id: req.request_id || '',
        restored_model_id: modelId,
        smoke_ok: true,
        domains: DOMAIN_IDS,
        errors: [],
      });
    } catch (err) {
      return JSON.stringify({
        version: '1',
        error: String(err && err.message ? err.message : err),
        code: String(err && err.message ? err.message : err).startsWith('feature_order_mismatch') ? 'feature_order_mismatch' : 'bridge_error',
        errors: [String(err && err.message ? err.message : err)],
      });
    }
  }

  // Register on globalThis
  globalThis.argosClassifier = {
    trainBatch,
    finishTraining,
    classifyBatch,
    serializeModel,
    restoreModel,
  };
})();
