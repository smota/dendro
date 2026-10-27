'use strict';
const test=require('node:test');const assert=require('node:assert/strict');const fs=require('node:fs');const vm=require('node:vm');
const path=require('node:path');
class Node {
 constructor(tag='div'){this.tag=tag;this.children=[];this.attrs={};this.listeners={};this._text='';this.style={setProperty(){}};this.value='';this.hidden=false;}
 set textContent(v){this._text=String(v);this.children=[];}get textContent(){return this._text+this.children.map(c=>c.textContent).join('');}
 append(...children){this.children.push(...children);}replaceChildren(...children){this._text='';this.children=children;if(this.tag==='select')this.value=children[0]?.value||'';}
 setAttribute(k,v){this.attrs[k]=String(v);}getAttribute(k){return this.attrs[k];}addEventListener(k,fn){this.listeners[k]=fn;}focus(){this.focused=true;}
}
const read=p=>fs.readFileSync(path.join(__dirname,p),'utf8');
function viewer(){const html=read('viewer.html'),nodes=new Map(),tabs=['pulse','flow','trace'].map(id=>{const n=new Node('button');n.setAttribute('aria-controls',id);return n;});
 const document={getElementById(id){if(!nodes.has(id))nodes.set(id,new Node(id==='seat'?'select':'div'));return nodes.get(id);},createElement:tag=>new Node(tag),createElementNS:(_,tag)=>new Node(tag),createTextNode:t=>{const n=new Node('#text');n.textContent=t;return n;},querySelectorAll:()=>tabs};
 document.getElementById('dendro-data').textContent='null';const context=vm.createContext({document,console});const script=html.match(/<script>\n([\s\S]*)<\/script>/)[1];vm.runInContext(script,context);return {context,nodes,tabs,run:code=>vm.runInContext(code,context)};}
test('scenario retains fractional returns, uses fleet totals, and handles endpoints',()=>{const context=vm.createContext({document:{addEventListener(){}}});vm.runInContext(read('app.js'),context);
 const calculate=x=>vm.runInContext(`modelScenario(${JSON.stringify(x)})`,context);const assumptions={seats:4,missions:25,tokensK:400,reworkPercent:25,costPerMillion:5,hours:2,reduction:20};const r=calculate(assumptions);
 assert.equal(r.returned,6.25);assert.equal(r.tokenMillions,2.5);assert.equal(r.cost,12.5);assert.equal(r.hours,2.5);assert.equal(r.capacity,5);assert.equal(r.missionsPerSeat,6.25);
 assert.equal(calculate({...assumptions,seats:50}).cost,12.5);assert.equal(calculate({...assumptions,reworkPercent:0}).cost,0);assert.equal(calculate({...assumptions,reduction:0}).hours,0);assert.equal(calculate({...assumptions,reworkPercent:100,reduction:100}).capacity,100);
});
test('synthetic demo renders SVG, timeline, local aliases, and numeric evidence',()=>{const v=viewer();assert.match(v.nodes.get('status').textContent,/Synthetic demo/);assert.equal(v.nodes.get('sankey').children[0].tag,'svg');assert.equal(v.nodes.get('timeline').children[0].tag,'svg');assert.match(v.nodes.get('inspection').textContent,/900,000/);assert.doesNotMatch(v.nodes.get('inspection').textContent,/builder/);
 const ribbon=v.nodes.get('sankey').children[0].children.find(c=>c.tag==='path');ribbon.listeners.click();assert.match(v.nodes.get('flow-detail').textContent,/Seat 1 → Seat 2: 2/);
});
test('real report envelope and trace inputs adapt without demo data',()=>{const v=viewer();v.run(`load({metric:'activity',payload:{product:'AI Fleet Intelligence',tokens:42,m1_by_seat:[{seat:'private-name',tokens:42,closed_items:0}] }},'test')`);assert.match(v.nodes.get('metrics').textContent,/42/);assert.match(v.nodes.get('timeline').textContent,/required/);assert.match(v.nodes.get('sankey').textContent,/No handoff/);assert.match(v.nodes.get('inspection').textContent,/Unavailable/);assert.doesNotMatch(v.nodes.get('inspection').textContent,/private-name|900,000/);
 v.run(`load({metric_id:'tokens',inputs:{tokens:7}},'trace')`);assert.match(v.nodes.get('metrics').textContent,/7/);assert.equal(v.nodes.get('seat').disabled,true);
});
test('projection excludes labels, secrets, source paths and nonnumeric injections',()=>{const v=viewer();const p=v.run(`project({product:'AI Fleet Intelligence',tokens:'<img src=x onerror=alert(1)>',m1_by_seat:[{seat:'sk-private-canary',tokens:5}],activity:[{seat:'sk-private-canary',source_key:'/private/canary',active_secs:-1}],command:'ghp_secret',metadata:{password:'canary'}})`);const serialized=JSON.stringify(p);assert.doesNotMatch(serialized,/sk-private|ghp_|canary|onerror|private/);assert.equal(p.total,null);assert.equal(p.activity[0].active_secs,null);assert.equal(p.seats[0].seat,'Seat 1');
});
test('failed import reports generic JSON error and does not silently use demo',async()=>{const v=viewer();await v.nodes.get('file').listeners.change({target:{files:[{size:4,text:async()=>'{bad'}],value:'x'}});assert.match(v.nodes.get('status').textContent,/Invalid JSON/);await v.nodes.get('file').listeners.change({target:{files:[{size:11*1024*1024}],value:'x'}});assert.match(v.nodes.get('status').textContent,/10 MiB/);assert.throws(()=>v.run(`project({unexpected:1})`),/Unsupported JSON/);
});
test('keyboard tabs change panel and selected state',()=>{const v=viewer();v.tabs[0].listeners.keydown({key:'ArrowRight',preventDefault(){}});assert.equal(v.tabs[1].attrs['aria-selected'],'true');assert.equal(v.nodes.get('flow').hidden,false);assert.equal(v.nodes.get('pulse').hidden,true);assert.equal(v.tabs[1].focused,true);});
test('life boundaries prevent running states carrying across unobserved intervals',()=>{const v=viewer();v.run(`load({product:'AI Fleet Intelligence',from:'2026-01-01T00:00:00Z',to:'2026-01-01T01:00:00Z',activity:[{seat:'a',observed_spans:[{from:'2026-01-01T00:00:00Z',to:'2026-01-01T00:20:00Z'},{from:'2026-01-01T00:40:00Z',to:'2026-01-01T01:00:00Z'}]}],activity_events:[{seat:'a',ts:'2026-01-01T00:00:00Z',state:'running'}]},'test')`);const rects=v.nodes.get('timeline').children[0].children.filter(x=>x.tag==='rect'&&x.attrs.fill==='#10b981');assert.equal(rects.length,1);assert.ok(Number(rects[0].attrs.width)<300);});
test('offline viewer has embedding marker and no remote dependencies; media preserved',()=>{const html=read('viewer.html');assert.match(html,/<script id="dendro-data" type="application\/json">null<\/script>/);assert.doesNotMatch(html,/<(?:script|link|img|iframe)[^>]+(?:src|href)="https?:/);assert.doesNotMatch(read('index.html'),/fonts\.google/);['assets/logo.jpg','assets/dendro-demo.mp4','assets/dendro-preview.gif'].forEach(p=>assert.ok(fs.statSync(path.join(__dirname,p)).size>0));});
test('pair heatmap separates explicit zero from unknown and renders execution / boundaries',()=>{const v=viewer();v.run(`load({product:'AI Fleet Intelligence',m2_pairs:[{from:'a',to:'b',handoffs:4,return_rate:0}],pair_medians:[{from:'a',to:'b',wait_median:0,exec_median:12,exec_samples:2}],ring_boundaries:{edges:[{from:'a',to:'b',boundary:'in_ring'}]}},'test')`);assert.match(v.nodes.get('heatmap').textContent,/4/);v.nodes.get('heatMetric').value='rate';v.nodes.get('heatMetric').listeners.change();assert.match(v.nodes.get('heatmap').textContent,/0/);assert.match(v.nodes.get('heatmap').textContent,/—/);v.nodes.get('heatMetric').value='exec';v.nodes.get('heatMetric').listeners.change();assert.match(v.nodes.get('heatmap').textContent,/12/);v.nodes.get('heatMetric').value='boundary';v.nodes.get('heatMetric').listeners.change();assert.match(v.nodes.get('heatmap').textContent,/in_ring: 1/);});
test('timestamped context-drop markers use only numeric counts and actual positions',()=>{const v=viewer();v.run(`load({product:'AI Fleet Intelligence',from:'2026-01-01T00:00:00Z',to:'2026-01-01T01:00:00Z',activity_events:[{seat:'a',ts:'2026-01-01T00:00:00Z',state:'running'}],context_compactions:{events:[{seat:'a',ts:'2026-01-01T00:30:00Z',pre_drop_context_tokens:325000,post_drop_context_tokens:34000,command:'private-canary'}]}},'test')`);const markers=v.nodes.get('timeline').children[0].children.filter(n=>n.tag==='line');assert.equal(markers.length,1);assert.equal(markers[0].attrs.x1,'545');assert.match(markers[0].textContent,/325,000 → 34,000/);assert.doesNotMatch(markers[0].textContent,/canary/);});
test('existing CLI handoff report and trace fixtures render using their real envelopes',{skip:!process.env.DENDRO_SITE_FIXTURE_DIR},()=>{const v=viewer();for(const name of ['loops.handoffs.candidate.json','loops.trace.candidate.json']){const raw=JSON.parse(fs.readFileSync(path.join(process.env.DENDRO_SITE_FIXTURE_DIR,name),'utf8'));v.context.fixture=raw;v.run(`load(fixture,'verified fixture')`);assert.match(v.nodes.get('status').textContent,/verified fixture/);assert.doesNotMatch(v.nodes.get('status').textContent,/Synthetic/);}});

test('committed native CLI fixture preserves actual numeric projection and aliased flows',()=>{const v=viewer();v.context.fixture=JSON.parse(read('fixtures/canonical-handoffs.json'));v.run("load(fixture,'native synthetic CLI fixture')");assert.equal(v.run('current.edges.length'),4);assert.equal(v.run('current.histogram[0].secs'),3600);assert.equal(v.run('current.pairs[0].exec_median'),9);assert.match(v.nodes.get('sankey').textContent,/Seat 1/);assert.match(v.nodes.get('concurrency').textContent,/3,600/);});
test('parallel return ribbons have distinct geometry and absent life remains unknown',()=>{const v=viewer();v.run("load({product:'AI Fleet Intelligence',from:'2026-01-01T00:00:00Z',to:'2026-01-01T01:00:00Z',handoff_edges:[{from:'a',to:'b'},{from:'a',to:'b',is_return:true}],activity_events:[{seat:'a',ts:'2026-01-01T00:00:00Z',state:'running'}]},'test')");const paths=v.nodes.get('sankey').children[0].children.filter(n=>n.tag==='path');assert.notEqual(paths[0].attrs.d,paths[1].attrs.d);assert.equal(v.nodes.get('timeline').children[0].children.filter(n=>n.tag==='rect'&&n.attrs.fill==='#10b981').length,0);});

test('source-backed container intervals render exact timeline positions without activity inference',()=>{const v=viewer();v.run("load({product:'AI Fleet Intelligence',from:'2026-01-01T00:00:00Z',to:'2026-01-01T01:00:00Z',advisory_idle_claimed:{intervals:[{seat:'private-seat',from:'2026-01-01T00:15:00Z',to:'2026-01-01T00:30:00Z',classification:'unparked_container_hold'}]}},'test')");const rects=v.nodes.get('timeline').children[0].children.filter(n=>n.tag==='rect'&&n.attrs.fill==='url(#container-hatch)');assert.equal(rects.length,1);assert.equal(Number(rects[0].attrs.x),332.5);assert.equal(Number(rects[0].attrs.width),212.5);assert.doesNotMatch(rects[0].textContent,/private-seat/);});

test('dashboards hub and multi-view suite exist, are offline standalone with live-sync and no remote dependencies',()=>{
 const files=[
  'dashboards/style.css',
  'dashboards/live-sync.js',
  'dashboards/index.html',
  'dashboards/views/executive_governance.html',
  'dashboards/views/seat_dynamics.html',
  'dashboards/views/handoff_flow.html',
  'dashboards/views/cost_and_loops.html'
 ];
 files.forEach(f=>{
  assert.ok(fs.existsSync(path.join(__dirname,f)),`File missing: ${f}`);
  const content=read(f);
  assert.ok(content.length>100,`File empty: ${f}`);
  assert.doesNotMatch(content,/<(?:script|link|img|iframe)[^>]+(?:src|href)="https?:/i,`Remote dependency found in ${f}`);
 });

 const indexHtml=read('dashboards/index.html');
 assert.match(indexHtml,/<script id="dendro-data" type="application\/json">/);
 assert.match(indexHtml,/attachLiveSync/);
 assert.match(indexHtml,/views\/executive_governance\.html/);
 assert.match(indexHtml,/views\/seat_dynamics\.html/);
 assert.match(indexHtml,/views\/handoff_flow\.html/);
 assert.match(indexHtml,/views\/cost_and_loops\.html/);
 assert.match(indexHtml,/\.\.\/viewer\.html/);

 ['executive_governance','seat_dynamics','handoff_flow','cost_and_loops'].forEach(v=>{
  const viewHtml=read(`dashboards/views/${v}.html`);
  assert.match(viewHtml,/<script id="dendro-data" type="application\/json">/);
  assert.match(viewHtml,/attachLiveSync/);
  assert.match(viewHtml,/\.\.\/index\.html/);
 });

 const siteIndex=read('index.html');
 assert.match(siteIndex,/dashboards\/index\.html/);
});
