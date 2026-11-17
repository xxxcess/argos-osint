/** Fleetworks Quiet Orbit v3 — continuous texture-backed 2.5D orbital motion.
 * The supplied revised reference artwork remains unchanged. Separate opaque prism layers
 * follow an inclined projected orbit, with depth sorting and sphere occlusion.
 * Molten contents flow inside a fixed glass sphere contour. No reveal or fade. */
let sharp; try { sharp=require('sharp'); } catch (error) {
 const modules=process.env.CODEX_PRIMARY_RUNTIME_NODE_MODULES;
 if(!modules) throw error; sharp=require(require('path').join(modules,'sharp'));
}
const fs=require('fs'),path=require('path');
const arg=Object.fromEntries(process.argv.slice(2).reduce((a,v,i,all)=>v.startsWith('--')?[...a,[v.slice(2),all[i+1]]]:a,[]));
const SOURCE=arg.source||path.join(__dirname,'..','..','..','source','fleetworks-liquid-metal-glass.png');
const variant=1,W=+(arg.width||800),H=Math.round(W*941/1672),FPS=20,N=+(arg.frames||160);
// Validated fixed orbit: sampled polygon clearance ~13.323px at 800px width.
const geometry={radiusRatio:1.3031175059952038,inclinationDegrees:66,liftAt800:38,prismScale:.70};
const OUT=arg.out||path.join(__dirname,'quiet-orbit');fs.mkdirSync(OUT,{recursive:true});
const sx=W/1672,sy=H/941,cx=834.5*sx,cy=435.5*sy,rx=208.5*sx,ry=208.5*sy;
const sat=[
 {b:[779,56,897,181],p:[[838,58],[895,179],[781,179]],tip:0},
 {b:[624,124,740,254],p:[[626,126],[738,190],[649,252]],tip:0},
 {b:[935,125,1053,254],p:[[1051,127],[1026,252],[937,190]],tip:0},
 {b:[480,285,623,393],p:[[482,294],[621,287],[576,391]],tip:0},
 {b:[1049,285,1192,394],p:[[1051,287],[1190,294],[1095,392]],tip:1}
].map((s,i)=>{let b=s.b.map((v,j)=>v*(j%2?sy:sx)),pts=s.p.map(([x,y])=>[x*sx,y*sy]),mx=pts.reduce((a,p)=>a+p[0],0)/3,my=pts.reduce((a,p)=>a+p[1],0)/3;return {...s,b,pts,mx,my,i,tipAngle:Math.atan2(pts[s.tip][1]-my,pts[s.tip][0]-mx)}});
const clamp=(v,a=0,b=1)=>Math.max(a,Math.min(b,v));
function smooth(a,b,x){let p=clamp((x-a)/(b-a));return p*p*(3-2*p)}
function sample(d,x,y,c){x=clamp(x,0,W-1.001);y=clamp(y,0,H-1.001);let xi=Math.floor(x),yi=Math.floor(y),u=x-xi,v=y-yi,i=(yi*W+xi)*3;return (d[i+c]*(1-u)+d[i+3+c]*u)*(1-v)+(d[i+W*3+c]*(1-u)+d[i+W*3+3+c]*u)*v}
function seg(x,y,x1,y1,x2,y2){let vx=x2-x1,vy=y2-y1,q=clamp(((x-x1)*vx+(y-y1)*vy)/(vx*vx+vy*vy));return Math.hypot(x-x1-q*vx,y-y1-q*vy)}
function polyInfo(x,y,pts){let inside=false,d=1e6;for(let k=0,j=pts.length-1;k<pts.length;j=k++){let [x1,y1]=pts[j],[x2,y2]=pts[k];if((y1>y)!=(y2>y)&&x<(x2-x1)*(y-y1)/(y2-y1)+x1)inside=!inside;d=Math.min(d,seg(x,y,x1,y1,x2,y2))}return {inside,d}}
(async()=>{
 const {data}=await sharp(SOURCE).resize(W,H,{fit:'fill'}).removeAlpha().raw().toBuffer({resolveWithObject:true});
 const base=Buffer.from(data);
 // Backing layer retains the original rocky crescent unchanged.
 // Only the original crown and sphere are removed for independent animation.
 // No crescent mask or hiding pass is used: clearance comes from the fixed orbit.
 for(let y=0;y<H;y++)for(let x=0;x<W;x++){
  let i=(y*W+x)*3,remove=((x-cx)/(rx+3))**2+((y-cy)/(ry+3))**2<1;
  for(const s of sat)if(x>=s.b[0]-3&&x<=s.b[2]+3&&y>=s.b[1]-3&&y<=s.b[3]+3)remove=true;
  if(remove)base[i]=base[i+1]=base[i+2]=0;
 }
 let traces=[],frameTimes=[];
 function drawSatellite(dst,tr){const {s,mx,my,z,sc,yaw,rot}=tr,cs=Math.cos(rot),sn=Math.sin(rot),bw=(s.b[2]-s.b[0]+6)*sc,bh=(s.b[3]-s.b[1]+6)*sc,R=Math.hypot(bw,bh),light=.91+.09*(z/tr.R+1)/2;
  for(let y=Math.max(0,Math.floor(my-R));y<Math.min(H,Math.ceil(my+R));y++)for(let x=Math.max(0,Math.floor(mx-R));x<Math.min(W,Math.ceil(mx+R));x++){
   const dx=(x-mx)/sc,dy=(y-my)/sc,tx=s.mx+(dx*cs+dy*sn)/yaw,ty=s.my-dx*sn+dy*cs;
   if(tx<s.b[0]-2||tx>s.b[2]+2||ty<s.b[1]-2||ty>s.b[3]+2)continue;
   if(((tx-cx)/(rx+2))**2+((ty-cy)/(ry+2))**2<1)continue;
   const p=polyInfo(tx,ty,s.pts),alpha=p.inside?1:clamp((1.1-p.d)/.8);if(alpha<=0)continue;
   const i=(y*W+x)*3;for(let c=0;c<3;c++)dst[i+c]=clamp(sample(data,tx,ty,c)*light*alpha+dst[i+c]*(1-alpha),0,255);
  }
 }
 const renderFrames=arg['phase-degrees']?[N*Number(arg['phase-degrees'])/360]:arg.proof?[0,Math.floor(N/4),Math.floor(N/2),Math.floor(3*N/4)]:Array.from({length:N},(_,i)=>i);
 for(const f of renderFrames){
  const start=Date.now(),phase=2*Math.PI*f/N,t=f/FPS,dst=Buffer.from(base);
  const planetShift=0;
  const tilt=geometry.inclinationDegrees*Math.PI/180;
  const planeRotation=0;
  const csP=Math.cos(planeRotation),snP=Math.sin(planeRotation),transforms=[];
  for(const s of sat){
   const theta=Math.PI/2+phase+s.i*2*Math.PI/5;
   const R=rx*geometry.radiusRatio;
   const ti=tilt,x3=R*Math.cos(theta),y3=R*Math.sin(theta)*Math.cos(ti),z=R*Math.sin(theta)*Math.sin(ti);
   const perspective=1/(1-z/(rx*7.5)),screenX=(x3*csP-y3*snP)*perspective,screenY=-(x3*snP+y3*csP)*perspective;
   const mx=cx+planetShift+screenX,my=cy-geometry.liftAt800*(W/800)+screenY,sc=perspective*geometry.prismScale,yaw=.82+.18*Math.abs(Math.sin(theta));
   const rot=Math.atan2(screenY,screenX)-s.tipAngle+.055*Math.sin(phase*2+s.i);
   transforms.push({s,mx,my,z,R,sc,yaw,rot});
  }
  transforms.sort((a,b)=>a.z-b.z);
  for(const tr of transforms)if(tr.z<0)drawSatellite(dst,tr);
  // The rear prisms are covered by a fixed glass sphere. Only its interior
  // texture coordinates flow; the outer glass contour never deforms.
  for(let y=Math.floor(cy-ry-2);y<=cy+ry+2;y++)for(let x=Math.floor(cx-rx-2+planetShift);x<=cx+rx+2+planetShift;x++){
   let nx=(x-cx-planetShift)/rx,ny=(y-cy)/ry,r2=nx*nx+ny*ny;if(r2>1.01)continue;
   const r=Math.sqrt(r2),fluid=1-smooth(.69,.95,r),a=clamp((1.006-r)*85),z=Math.sqrt(Math.max(0,1-r2));
   let angle=.105*Math.sin(phase),ax=nx*Math.cos(angle)+z*Math.sin(angle),ay=ny;
   const dx=.055*Math.sin(phase+ny*3.8+nx*1.2)+.018*Math.sin(2*phase-nx*5),dy=.041*Math.cos(phase+nx*4.2-ny*1.6);
   ax=nx+(ax-nx)*fluid+dx*fluid;ay=ny+dy*fluid;
   let ar=Math.hypot(ax,ay);if(fluid>0&&ar>.985){ax*=.985/ar;ay*=.985/ar}
   const light=1;
   const i=(y*W+x)*3;for(let c=0;c<3;c++)dst[i+c]=clamp(sample(data,cx+ax*rx,cy+ay*ry,c)*light*a+dst[i+c]*(1-a),0,255);
  }
  for(const tr of transforms)if(tr.z>=0)drawSatellite(dst,tr);
  await sharp(dst,{raw:{width:W,height:H,channels:3}}).png().toFile(path.join(OUT,arg['phase-degrees']?'clearance-proof.png':String(f+1).padStart(4,'0')+'.png'));
  traces.push({frame:f+1,phase,t,satellites:transforms.map(q=>({id:q.s.i+1,x:+q.mx.toFixed(3),y:+q.my.toFixed(3),z:+q.z.toFixed(3),front:q.z>=0}))});
  frameTimes.push(Date.now()-start);if(f%32===0)console.log(JSON.stringify({variant,frame:f+1,total:N,ms:frameTimes.at(-1)}));
 }
 fs.writeFileSync(path.join(OUT,'motion.json'),JSON.stringify({variant,geometry,source:path.basename(SOURCE),width:W,height:H,fps:FPS,frames:N,renderedFrameCount:renderFrames.length,duration:N/FPS,loop:'Full 2π orbit; periodic flow, transforms and lighting; no intro/outro fade.',fullMaterialsAtFrame1:true,satelliteCount:5,occlusion:'Negative-depth prisms behind opaque planet; positive-depth prisms in front; each set depth-sorted. Crescent is untouched; no crescent mask or collision hiding is used.',meanFrameMs:frameTimes.reduce((a,b)=>a+b,0)/N},null,2));
 fs.writeFileSync(path.join(OUT,'orbital-trace.json'),JSON.stringify(traces));
 console.log(OUT);
})();
