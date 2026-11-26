// This wrapper has its own DTO and never enters the article cache.
globalThis.argosConversation = function(text) {
  const MAX = 12;
  function spans(pattern) {
    const hits = [];
    for (const match of text.matchAll(pattern)) {
      hits.push({start: match.index, end: match.index + match[0].length});
      if (hits.length === MAX) break;
    }
    return hits;
  }
  const hints = [];
  for (const sentence of nlp(text).json({terms: true, offset: true})) {
    for (const term of sentence.terms || []) {
      const tags = term.tags || [];
      const kind = tags.includes('Person') || tags.includes('Place') ||
        tags.includes('Organization') ? 'entity' :
        tags.includes('Verb') ? 'verb' :
        tags.includes('Noun') ? 'noun' : null;
      if (kind && term.offset && hints.length < MAX) {
        hints.push({kind, span: {start: term.offset.start, end: term.offset.start + term.offset.length}});
      }
    }
  }
  return JSON.stringify({
    hints,
    subjects: spans(/\b(?:actor search(?: index)?|actor index|search index|actor catalog|catalog|index|api gateway|gateway|datastore|database|backend|workstation|readiness|status checks?|cpu|processor|memory|ram|disk|resources|jobs?|cycles?|assistant|model|classifier|inference|training)\b/gi),
    question_words: spans(/\b(?:who|what|which|when|why|where|how)\b/gi),
    references: spans(/\b(?:it|this|that|those|them|ones?|former|latter)\b/gi),
    corrections: spans(/^\s*(?:actually|rather|i meant|i mean|no|not)\b/gi),
    negations: spans(/\b(?:not|never|without|do not|don['’]t|no|can['’]t|isn['’]t)\b/gi),
    exclusions: spans(/(?:[,;. ]+)(?:please )?(?:no (?:advice|suggestions|recommendations)|without (?:advice|suggestions|recommendations)|(?:do not|don['’]t) (?:suggest (?:anything|a restart)|recommend (?:anything|a restart)|(?:restart|stop|start|cancel|retry|change) (?:it|anything|(?:the )?(?:backend|job|index|service))))(?=$|[.?!;])/gi),
    temporal: spans(/\b(?:between \d{4}-\d{2}-\d{2} and \d{4}-\d{2}-\d{2}|(?:(?:since|on|before|after|until)\s+)?\d{4}-\d{2}-\d{2}|(?:since|before|after|until)\s+(?:yesterday|today|last\s+(?:week|month)|\S+)|(?:in\s+the\s+)?(?:last|past)\s+\d+\s+(?:minutes?|hours?|days?)|(?:right\s+now|currently|now|today|yesterday|last\s+(?:week|month)|recently|historically))\b/gi),
  });
};
