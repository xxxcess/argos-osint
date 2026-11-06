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

  const BRIDGE_VERSION = 'argos-classifier-bridge-v1';
  const PAYLOAD_SCHEMA_VERSION = 'classifier-payload-v1';
  const TAXONOMY_VERSION = 'strategic-domains-v1';
  const FEATURE_SCHEMA_VERSION = 'classifier-features-v2';
  const ALGORITHM_VERSION = 'natural-bayes-binary-ovr-v1';
  const PREPROCESSING_VERSION = 'natural-en-keepstops-v1';

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
      for (let domain = 0; domain < 6; domain++) {
        const added = rows.filter(row => row.membership[domain] !== 'Unknown').length;
        if (ctx.classifiers[domain].classifier.totalExamples - 1 + ctx.classifiers[domain].docs.length - ctx.classifiers[domain].lastAdded + added > 50000) throw new Error('training_document_bound');
      }
      ctx.round++;

      let accepted = 0;
      let skipped = 0;
      let tokenBytes = 0;
      let vocabNew = 0;

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
        const tokens = ctx.stemmer.tokenizeAndStem(text, true);
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
          const prevOrderLen = requireFeatureOrder(classifier, DOMAIN_IDS[d]).length;

          if (mem === 'Present') {
            classifier.addDocument(tokens, 'present');
            rowContributed = true;
          } else if (mem === 'Absent') {
            classifier.addDocument(tokens, 'absent');
            rowContributed = true;
          }
          // If 'Unknown', omitted entirely (masked partial label)

          const newOrderLen = requireFeatureOrder(classifier, DOMAIN_IDS[d]).length;
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

      const vocabSizes = ctx.classifiers.map((c) =>
        (() => { if (!Array.isArray(c.featureOrder)) throw new Error('feature_order_mismatch'); return c.featureOrder.length; })()
      );

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
      const vocabSizes = ctx.classifiers.map((classifier, index) => requireFeatureOrder(classifier, DOMAIN_IDS[index]).length);
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
        const tokens = ctx.stemmer.tokenizeAndStem(text, true);
        const uniqueTokens = Array.from(new Set(tokens));

        const domainScores = [];

        for (let d = 0; d < 6; d++) {
          const domain = DOMAIN_IDS[d];
          const classifier = ctx.classifiers[d];

          // Known vs OOV features against this model's vocabulary
          let knownCount = 0;
          let oovCount = 0;
          const featuresMap = classifier.features || {};

          for (const tok of uniqueTokens) {
            if (featuresMap[tok] !== undefined) {
              knownCount++;
            } else {
              oovCount++;
            }
          }

          // Evaluate log scores
          const presentLog = classifier.logScore(tokens, 'present');
          const absentLog = classifier.logScore(tokens, 'absent');
          const rawLogOdds = presentLog - absentLog;

          domainScores.push({
            domain,
            present_log_score: presentLog,
            absent_log_score: absentLog,
            raw_log_odds: rawLogOdds,
            known_feature_count: knownCount,
            oov_count: oovCount,
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
          c.features[featureOrder[i]] = 1;
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
