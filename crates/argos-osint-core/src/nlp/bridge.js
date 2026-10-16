// Input and offsets are never normalized. One sentence/term tree per chunk.
nlp.extend(compromiseDates);
globalThis.argosAnnotate = function(text) {
  const tree = nlp(text).json({terms:true, offset:true});
  const sentences=[], mentions=[], dates=[], numbers=[];
  for (const sentence of tree) {
    const terms=sentence.terms || [];
    const offset=sentence.offset;
    if(offset && offset.length>0) sentences.push({start:offset.start,end:offset.start+offset.length});
    for(const [kind,tags,target] of [
      ['person',['Person'],mentions],['place',['Place'],mentions],
      ['organization',['Organization'],mentions],['acronym',['Acronym'],mentions],
      ['hashtag',['HashTag'],mentions],['mention',['AtMention'],mentions],['date',['Date'],dates],
      ['number',['Value','Money','Percent','NumericValue'],numbers]]) {
      let run=null;
      for(const t of terms) {
        const tagged=(t.tags||[]).some(tag=>tags.includes(tag));
        const scale=kind==='number' && /^(billion|million|trillion|thousand|percent|dollars|euros|pounds|km|kg|miles|meters|metres|kilometers|kilometres|tons|tonnes|lbs|seconds|minutes|hours|days|weeks|months|years)$/i.test(t.text);
        if ((tagged || (run && scale)) && t.offset) {
          const end=t.offset.start+t.offset.length;
          if(run) run.end=end;
          else run={start:t.offset.start,end,kind};
        } else if(run) {target.push(run);run=null;}
      }
      if(run) target.push(run);
    }
  }
  return JSON.stringify({sentences,mentions,dates,numbers});
};
globalThis.argosDate = function(phrase, today, timezone, dateOrder) {
  return JSON.stringify(nlp(phrase).dates({today,timezone,dateOrder}).json());
};
// Lexical cleanup only. Require a confidently identified entity after/before
// verbal context, and preserve ambiguous proper names and branded phrases.
globalThis.argosActorNames = function(input) {
  return JSON.stringify(JSON.parse(input).map(text => {
    const sentences=nlp(text).json({terms:true,offset:true});
    if(sentences.length!==1) return text;
    const terms=sentences[0].terms || [];
    const has=(t,tag)=>(t.tags||[]).includes(tag);
    const entity=t=>has(t,'Organization') || has(t,'Person') || has(t,'Place');
    const verb=t=>has(t,'Verb') && !has(t,'ProperNoun') && !has(t,'Modal');
    let start=0,end=terms.length;
    while(start<end && verb(terms[start])) start++;
    if(start>0) while(start<end && has(terms[start],'Determiner')) start++;
    while(end>start && verb(terms[end-1])) end--;
    const kept=terms.slice(start,end);
    if(!kept.length || !kept.some(entity)) return text;
    // A leading verb before a person is ambiguous (e.g. Grant Williams).
    if(start>0 && kept.some(t=>has(t,'Person')) && !kept.some(t=>has(t,'Organization'))) return text;
    const first=kept[0].offset,last=kept[kept.length-1].offset;
    return first && last ? text.slice(first.start,last.start+last.length) : text;
  }));
};
