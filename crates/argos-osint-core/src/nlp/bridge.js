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
