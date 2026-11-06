/**
 * Argos Natural Bayes Adaptations
 *
 * Explicit deterministic transformations applied to vendored upstream
 * Natural and Apparatus files to produce QuickJS-compatible code:
 * - Remove Node filesystem (fs) requires and persistence methods (save, load)
 * - Exclude classifier_train_parallel (os, webworker-threads)
 * - Implement monotonically append-only featureOrder for deterministic incremental training
 * - Implement log-space scoring (logScore) matching apparatus probabilityOfClass
 * - Use Object.setPrototypeOf instead of __proto__
 */

'use strict';

/**
 * Apply adaptations to an upstream file by its relative path within upstream/.
 * @param {string} relPath - e.g. "natural/lib/natural/classifiers/classifier.js"
 * @param {string} source - Original file content
 * @returns {string} Adapted file content
 */
function adaptSource(relPath, source) {
  const normalized = relPath.replace(/\\/g, '/');

  if (normalized.endsWith('natural/lib/natural/classifiers/classifier.js')) {
    let code = source;

    // 1. Remove fs require
    code = code.replace("const fs = require('fs')\n", '');

    // 2. Remove parallelTrainer require
    code = code.replace(
      "const parallelTrainer = require('./classifier_train_parallel')\n",
      ''
    );

    // 3. In constructor, remove parallel training methods and initialize featureOrder
    const constructorOld = `    // Add methods for parallel training
    this.Threads = parallelTrainer.Threads
    this.trainParallel = parallelTrainer.trainParallel
    this.retrainParallel = parallelTrainer.retrainParallel
    this.trainParallelBatches = parallelTrainer.trainParallelBatches`;
    const constructorNew = `    this.featureOrder = []
    this.featureIndex = []`;
    code = code.replace(constructorOld, constructorNew);

    // 4. In addDocument, maintain append-only featureOrder
    const featureCountOld = `    for (let i = 0; i < text.length; i++) {
      const token = text[i]
      this.features[token] = (this.features[token] || 0) + 1
    }`;
    const featureCountNew = `    for (let i = 0; i < text.length; i++) {
      const token = text[i]
      if (this.features[token] === undefined) {
        this.features[token] = 1
        const index = this.featureOrder.length
        this.featureOrder.push(token)
        this.featureIndex.push({ token, index })
      } else {
        this.features[token]++
      }
    }`;
    code = code.replace(featureCountOld, featureCountNew);

    // 5. In textToFeatures, iterate append-only featureOrder
    const textToFeaturesOld = `  textToFeatures (observation) {
    const features = []

    if (typeof observation === 'string') { observation = this.stemmer.tokenizeAndStem(observation, this.keepStops) }

    for (const feature in this.features) {
      if (observation.indexOf(feature) > -1) {
        features.push(1)
      } else {
        features.push(0)
      }
    }

    return features
  }`;
    const textToFeaturesNew = `  textToFeatures (observation) {
    const features = []

    if (typeof observation === 'string') { observation = this.stemmer.tokenizeAndStem(observation, this.keepStops) }

    if (!Array.isArray(this.featureOrder)) { throw new Error('feature_order_mismatch') }
    const order = this.featureOrder
    if (!Array.isArray(this.featureIndex) || this.featureIndex.length !== order.length) { throw new Error('feature_order_mismatch') }
    for (let i = 0; i < order.length; i++) {
      const feature = order[i]
      const binding = this.featureIndex[i]
      if (!binding || binding.index !== i || binding.token !== feature) { throw new Error('feature_order_mismatch') }
      if (observation.indexOf(feature) > -1) {
        features.push(1)
      } else {
        features.push(0)
      }
    }

    return features
  }`;
    code = code.replace(textToFeaturesOld, textToFeaturesNew);

    // 6. In train, support trainStep for bounded incremental fitting
    const trainOld = `  train () {
    const totalDocs = this.docs.length
    for (let i = this.lastAdded; i < totalDocs; i++) {
      const features = this.textToFeatures(this.docs[i].text)
      this.classifier.addExample(features, this.docs[i].label)
      this.emit('trainedWithDocument', { index: i, total: totalDocs, doc: this.docs[i] })
      this.lastAdded++
    }
    this.classifier.train()
    this.emit('doneTraining', true)
  }`;
    const trainNew = `  trainStep (maxDocs) {
    const totalDocs = this.docs.length
    const limit = (typeof maxDocs === 'number' && maxDocs > 0)
      ? Math.min(totalDocs, this.lastAdded + maxDocs)
      : totalDocs
    for (let i = this.lastAdded; i < limit; i++) {
      const features = this.textToFeatures(this.docs[i].text)
      this.classifier.addExample(features, this.docs[i].label)
      this.emit('trainedWithDocument', { index: i, total: totalDocs, doc: this.docs[i] })
      this.lastAdded++
    }
    if (this.lastAdded >= totalDocs) {
      this.classifier.train()
      this.emit('doneTraining', true)
      return true
    }
    return false
  }

  train () {
    this.trainStep()
  }`;
    code = code.replace(trainOld, trainNew);

    // 7. In restore, preserve and restore featureOrder
    const restoreOld = `  static restore (classifier, stemmer) {
    classifier.stemmer = stemmer || PorterStemmer
    return classifier
  }`;
    const restoreNew = `  static restore (classifier, stemmer) {
    classifier.stemmer = stemmer || PorterStemmer
    if (!Array.isArray(classifier.featureOrder) || classifier.featureOrder.length === 0 || new Set(classifier.featureOrder).size !== classifier.featureOrder.length) {
      throw new Error('feature_order_mismatch')
    }
    if (!Array.isArray(classifier.featureIndex) || classifier.featureIndex.length !== classifier.featureOrder.length || classifier.featureIndex.some((binding, index) => !binding || binding.index !== index || binding.token !== classifier.featureOrder[index])) {
      throw new Error('feature_order_mismatch')
    }
    if (classifier.features && (Object.keys(classifier.features).length !== classifier.featureOrder.length || classifier.featureOrder.some(token => !Object.prototype.hasOwnProperty.call(classifier.features, token)))) {
      throw new Error('feature_order_mismatch')
    }
    if (!classifier.features && classifier.featureOrder) {
      classifier.features = {}
      for (let i = 0; i < classifier.featureOrder.length; i++) {
        classifier.features[classifier.featureOrder[i]] = 1
      }
    }
    return classifier
  }`;
    code = code.replace(restoreOld, restoreNew);

    // 8. Remove removeDocument and fs-based save, load, saveTo, loadFrom
    const removeDocBlock = `  removeDocument (text, classification) {
    const docs = this.docs
    let doc
    let pos

    if (typeof text === 'string') {
      text = this.stemmer.tokenizeAndStem(text, this.keepStops)
    }

    for (let i = 0, ii = docs.length; i < ii; i++) {
      doc = docs[i]
      if (doc.text.join(' ') === text.join(' ') &&
          doc.label === classification) {
        pos = i
      }
    }

    // Remove if there's a match
    if (!isNaN(pos)) {
      this.docs.splice(pos, 1)

      for (let i = 0, ii = text.length; i < ii; i++) {
        delete this.features[text[i]]
      }
    }
  }\n\n`;
    code = code.replace(removeDocBlock, '');

    const fsMethodsOld = `  save (filename, callback) {
    const data = JSON.stringify(this)
    const classifier = this
    fs.writeFile(filename, data, 'utf8', function (err) {
      if (callback) {
        callback(err, err ? null : classifier)
      }
    })
  }

  static load (filename, stemmer, callback) {
    if (!callback) {
      return
    }
    fs.readFile(filename, 'utf8', function (err, data) {
      if (err) {
        callback(err, null)
      } else {
        const classifier = JSON.parse(data)
        callback(err, Classifier.restore(classifier, stemmer))
      }
    })
  }

  async saveTo (storageBackend) {
    return await storageBackend.store(this)
  }

  static async loadFrom (key, storageBackend) {
    const obj = await storageBackend.retrieve(key)
    Object.setPrototypeOf(obj, Classifier.prototype)
    return obj
  }\n\n`;
    code = code.replace(fsMethodsOld, '');

    return code;
  }

  if (normalized.endsWith('natural/lib/natural/classifiers/bayes_classifier.js')) {
    let code = source;

    // Remove load and loadFrom methods
    const loadMethodsOld = `  static load (filename, stemmer, callback) {
    Classifier.load(filename, stemmer, function (err, classifier) {
      if (err) {
        return callback(err)
      } else {
        callback(err, BayesClassifier.restore(classifier, stemmer))
      }
    })
  }

  static async loadFrom (key, stemmer, storageBackend) {
    const classifier = await Classifier.loadFrom(key, storageBackend)
    return BayesClassifier.restore(classifier, stemmer)
  }\n`;
    code = code.replace(loadMethodsOld, '');

    // Add logScore helper on prototype
    const restoreEnd = `    classifier.classifier = ApparatusBayesClassifier.restore(classifier.classifier)
    return classifier
  }`;
    const logScoreAddition = `    classifier.classifier = ApparatusBayesClassifier.restore(classifier.classifier)
    return classifier
  }

  logScore (observation, label) {
    return this.classifier.logScore(this.textToFeatures(observation), label)
  }`;
    code = code.replace(restoreEnd, logScoreAddition);

    return code;
  }

  if (normalized.endsWith('apparatus/lib/apparatus/classifier/bayes_classifier.js')) {
    let code = source;

    // 1. Add logScore implementation
    const probFunc = `function probabilityOfClass(observation, label) {`;
    const logScoreFunc = `function logScore(observation, label) {
    if (!this.classFeatures[label] || !this.classTotals[label] || this.totalExamples <= 0) {
        return -Infinity;
    }
    var prob = Math.log(this.classTotals[label] / this.totalExamples);

    if (observation instanceof Array) {
        var i = observation.length;
        while (i--) {
            if (observation[i]) {
                var count = this.classFeatures[label][i] || this.smoothing;
                prob += Math.log(count / this.classTotals[label]);
            }
        }
    } else {
        for (var key in observation) {
            var count = this.classFeatures[label][observation[key]] || this.smoothing;
            prob += Math.log(count / this.classTotals[label]);
        }
    }
    return prob;
}

function probabilityOfClass(observation, label) {`;
    code = code.replace(probFunc, logScoreFunc);

    // 2. Register logScore on prototype
    const protoReg = `BayesClassifier.prototype.probabilityOfClass = probabilityOfClass;`;
    const protoRegNew = `BayesClassifier.prototype.probabilityOfClass = probabilityOfClass;\nBayesClassifier.prototype.logScore = logScore;`;
    code = code.replace(protoReg, protoRegNew);

    // 3. Use Object.setPrototypeOf in restore
    code = code.replace(
      '     classifier.__proto__ = BayesClassifier.prototype;',
      '     Object.setPrototypeOf(classifier, BayesClassifier.prototype);'
    );

    // 4. Expose BayesClassifier property for compatibility
    code = code.replace(
      'module.exports = BayesClassifier;',
      'BayesClassifier.BayesClassifier = BayesClassifier;\nmodule.exports = BayesClassifier;'
    );

    return code;
  }

  return source;
}

module.exports = {
  adaptSource
};
