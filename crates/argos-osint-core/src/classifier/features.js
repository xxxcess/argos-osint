/** Versioned data-only feature extraction shared by fitting and inference.
 * Byte spans refer to the original field. Punctuation/field boundaries break phrases.
 * Natural receives binary feature presence, never fractional TF-IDF weights.
 */
'use strict';
(() => {
  const VERSION = 'natural-en-phrases-v2';
  const MAX_FEATURES = 4096;
  const NOISE = new Set(['a','an','the','and','or','of','to','in','on','at','by','for','with','from','as','is','are','was','were','be','been','being','it','its','this','that','these','those','which','who']);
  const byteLength = text => {
    let bytes = 0;
    for (const ch of text) { const code = ch.codePointAt(0); bytes += code < 128 ? 1 : code < 2048 ? 2 : code < 65536 ? 3 : 4; }
    return bytes;
  };
  function extract(title, description) {
    if (typeof title !== 'string' || typeof description !== 'string' || byteLength(title)+byteLength(description)>32768) throw new Error('feature_input_bound');
    const features = new Map();
    const add = (id, field, start, end, phrase) => {
      if (!features.has(id)) {
        if (features.size >= MAX_FEATURES) throw new Error('feature_count_bound');
        features.set(id, {id, field, start_byte:start, end_byte:end, phrase});
      }
    };
    for (const [field, text] of [['title',title],['description',description]]) {
      if (text.length === 0) continue;
      const expression = /[A-Za-z]+(?:\.[A-Za-z]+)+\.?|[A-Za-z0-9]+(?:[-'’][A-Za-z0-9]+)*/g;
      let previous = null;
      let cursor = 0;
      let byteCursor = 0;
      const tokenizer = new globalThis.argosNaturalBayes.RegexpTokenizer({pattern:expression,gaps:false});
      for (const original of tokenizer.tokenize(text) || []) {
        const index = text.indexOf(original,cursor);
        const normalized = original.toLowerCase().replace(/’/g,"'");
        // Acronyms, CVEs, numbers and compound identifiers keep their identity.
        const protectedToken = /[0-9.-]/.test(original) || /^[A-Z]{2,}$/.test(original);
        const token = protectedToken ? normalized : globalThis.argosNaturalBayes.PorterStemmer.stem(normalized);
        const start = byteCursor + byteLength(text.slice(cursor,index));
        const end = start + byteLength(original);
        const current = {token, noise:NOISE.has(normalized), start, end, end_index:index+original.length};
        if (previous && /^[ \t]+$/.test(text.slice(previous.end_index,index))) {
          // Form phrases before removing grammatical noise. No skipped-token phrases.
          if (!previous.noise || !current.noise) {
            const pair = globalThis.argosNaturalBayes.NGrams.bigrams([previous.token, token])[0];
            add('b:'+JSON.stringify(pair),field,previous.start,end,true);
          }
        }
        if (!NOISE.has(normalized)) add('u:'+JSON.stringify(token),field,start,end,false);
        if (/n't$/.test(normalized)) add('u:"not"',field,start,end,false);
        previous = current;
        cursor = current.end_index;
        byteCursor = end;
      }
    }
    return Array.from(features.values());
  }
  globalThis.argosClassifierFeatures = Object.freeze({version:VERSION, max_features:MAX_FEATURES, extract, byteLength});
})();
