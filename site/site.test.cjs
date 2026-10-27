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
 assert.match(indexHtml,/views\/message_mix\.html/);
 assert.match(indexHtml,/views\/subagent_lineage\.html/);
 assert.match(indexHtml,/views\/security_posture\.html/);
 assert.match(indexHtml,/views\/process_resources\.html/);
 assert.match(indexHtml,/\.\.\/viewer\.html/);

 ['executive_governance','seat_dynamics','handoff_flow','cost_and_loops','message_mix','subagent_lineage','security_posture','process_resources'].forEach(v=>{
  const viewHtml=read(`dashboards/views/${v}.html`);
  assert.match(viewHtml,/<script id="dendro-data" type="application\/json">/);
  assert.match(viewHtml,/attachLiveSync/);
  assert.match(viewHtml,/\.\.\/index\.html/);
 });

 const siteIndex=read('index.html');
 assert.match(siteIndex,/dashboards\/index\.html/);
});

test('v0.8.0 layer abstraction, six insights, and eight dashboard projections',()=>{
 const indexHtml=read('dashboards/index.html');
 const siteIndex=read('index.html');
 const viewerHtml=read('viewer.html');
 assert.match(siteIndex,/v0\.8\.0/);
 assert.match(indexHtml,/0\.8\.0/);
 assert.doesNotMatch(indexHtml,/0\.7\.0/);
 ['All Layers','Model Layer','Harness Layer','Coordination &amp; Lineage','Environment &amp; Security','Platform Adapter (OpenRig)'].forEach(label=>assert.match(indexHtml,new RegExp(label.replace(/[()]/g,'\\$&'))));
 assert.match(indexHtml,/Compiled Insights Strip \(insight_v1\)/);
 ['I02','I04','I08','I09','I14','I18'].forEach(id=>assert.match(indexHtml,new RegExp(id)));
 ['model\\.tokens\\.direct','model\\.cost','harness\\.action_failure','work\\.lineage_shape','environment\\.resources','coverage\\.source'].forEach(name=>assert.match(indexHtml,new RegExp(name)));
 assert.match(indexHtml,/Model Token &amp; Cost Accounting \(I02 \/ I04\)/);
 assert.match(indexHtml,/Work Handoff &amp; Rework Friction/);
 assert.match(indexHtml,/Actor Activity &amp; Session Yield/);
 assert.match(indexHtml,/Governance &amp; Verification Scorecard/);
 ['cost_and_loops','handoff_flow','seat_dynamics','executive_governance'].forEach(view=>{
  assert.match(read(`dashboards/views/${view}.html`),/OpenRig Adapter Binding/);
 });
 const queries={
  message_mix:'v_message_mix',
  subagent_lineage:'v_subagent_lineage',
  security_posture:'v_security_posture',
  process_resources:'v_process_resources'
 };
 Object.entries(queries).forEach(([view,sql])=>{
  const html=read(`dashboards/views/${view}.html`);
  assert.match(html,new RegExp(sql));
  assert.match(html,/live-sync\.js/);
  assert.match(html,/style\.css/);
 });
 assert.match(read('dashboards/views/message_mix.html'),/I02/);
 assert.match(read('dashboards/views/message_mix.html'),/I08/);
 assert.match(read('dashboards/views/subagent_lineage.html'),/I09/);
 assert.match(read('dashboards/views/security_posture.html'),/effective_authority/);
 assert.match(read('dashboards/views/process_resources.html'),/I14/);
 assert.match(read('dashboards/views/process_resources.html'),/I18/);
 assert.match(viewerHtml,/Read your agentic stack across Model, Harness, Coordination, and Environment layers — with OpenRig adapter bindings when present/);
 ['id="pulse"','id="flow"','id="insights"','id="bands"','id="tokens"','id="lineage"','id="environment"','id="security"','id="inspection"'].forEach(id=>assert.match(viewerHtml,new RegExp(id)));
 const v=viewer();
 const insights=v.nodes.get('insight-list').textContent;
 ['I02','I04','I08','I09','I14','I18','model.tokens.direct','model.cost','harness.action_failure','work.lineage_shape','environment.resources','coverage.source','model','harness','coordination','environment'].forEach(token=>assert.match(insights,new RegExp(token.replace(/\./g,'\\.'))));
 assert.match(v.nodes.get('lineage').textContent,/I09/);
 assert.match(v.nodes.get('environment').textContent,/I14/);
 assert.match(v.nodes.get('security').textContent,/sandbox_policy/);
 const root=path.join(__dirname,'..');
 const templates=[
  ['message_mix.yaml','message_mix','v_message_mix'],
  ['subagent_lineage.yaml','subagent_lineage','v_subagent_lineage'],
  ['security_posture.yaml','security_posture','v_security_posture'],
  ['process_resources.yaml','process_resources','v_process_resources'],
  ['cost_and_loops.yaml','token_lineage','v_mission_token_burn'],
  ['handoff_flow.yaml','handoff_flow','v_handoff_rework_analysis'],
  ['seat_dynamics.yaml','seat_dynamics','v_seat_idle_ranking'],
  ['executive_governance.yaml','executive_governance','v_executive_governance']
 ];
 const caveat='All-time counts — window_from/window_to label the snapshot but do not filter.';
 ['message_mix','subagent_lineage','security_posture','process_resources'].forEach(view=>{
  assert.match(read(`dashboards/views/${view}.html`),new RegExp(caveat.replace(/[.*+?^${}()|[\]\\]/g,'\\$&')));
 });
 assert.match(indexHtml,new RegExp(caveat.replace(/[.*+?^${}()|[\]\\]/g,'\\$&')));
 const resources=read('dashboards/views/process_resources.html');
 assert.match(resources,/Process attribution/);
 assert.doesNotMatch(resources,/Isolation stage/);
 assert.match(resources,/rss_unit === 'bytes'/);
 assert.match(resources,/Units are never mixed/);
 const doc=fs.readFileSync(path.join(root,'docs/NAO_SEMANTIC.md'),'utf8');
 templates.forEach(([file,id,view])=>{
  const yaml=fs.readFileSync(path.join(root,'templates/nao/dashboards',file),'utf8');
  assert.match(yaml,new RegExp(`id: ${id}`));
  assert.match(yaml,new RegExp(view));
  assert.match(doc,new RegExp(view));
  assert.match(doc,new RegExp(file.replace('.','\\.')));
 });
 assert.match(doc,/4GB DuckDB memory cap/);
 assert.match(doc,/CPU deltas and max sampled RSS/);
 assert.match(fs.readFileSync(path.join(root,'crates/dendro-core/src/semantic.rs'),'utf8'),/CPU deltas and max sampled RSS/);
});

test('viewer strips untrusted free-text secrets from insights and security',()=>{
 const v=viewer();
 const rendered=()=>[...v.nodes.values()].map(node=>node.textContent).join('\n');
 const demoHtml=rendered();
 assert.ok(!/redacted/i.test(demoHtml));
 assert.match(demoHtml,/actions/);
 assert.match(demoHtml,/sessions/);
 const canaries=['sk-ant-api03-AbC123','ghp_canaryAbC123','sk-live-abc123','xai-canaryToken99','unlisted_custom_slug'];
 for(const canary of canaries){
  const payload=structuredClone(v.run('DEMO'));
  payload.insights.forEach(row=>{
   row.status={state:'partial',reason:canary};
   row.population.what=canary;
   row.measures=[{id:canary,name:canary,value:1,unit:canary}];
  });
  payload.security={status:{state:'partial',reason:canary},population:{what:canary,n:1},counts:[{kind:canary,value:canary,n:1},{kind:'sandbox_policy',value:canary,n:1},{kind:'approved_prefix_count',value:canary,n:1}]};
  v.context.payload=payload;
  v.run("load(payload,'privacy check')");
  const shown=rendered()+(v.run('JSON.stringify(project(payload))')||'');
  assert.doesNotMatch(shown,new RegExp(canary.replace(/[.*+?^${}()|[\]\\]/g,'\\$&')));
 }
 v.run("load(DEMO,'Synthetic demo')");
 const secret='super-secret-value';
 const payload={
  product:'AI Fleet Intelligence',
  insights:[{
   id:'I02',name:secret,version:'1.0.0',domain:'model',status:{state:'computed',reason:secret},
   population:{what:secret,n:1},
   measures:[{id:'I02.input_uncached',name:secret,value:1,unit:secret}]
  },{
   id:secret,name:'model.cost',version:'1.0.0',domain:'model',status:{state:'computed'},
   population:{what:'usage_facts',n:1},measures:[]
  }],
  lineage:{id:'I09',name:secret,domain:'coordination',status:{state:'partial',reason:'unresolved parent session'},population:{what:secret,n:2},measures:[{id:'I09.max_depth',name:'max_depth',value:1,unit:secret}]},
  security:{status:{state:'computed'},population:{what:secret,n:1},counts:[{kind:secret,value:'workspace-write',n:1},{kind:'sandbox_policy',value:secret,n:1},{kind:'sandbox_policy',value:'workspace-write',n:2}]}
 };
 v.context.payload=payload;
 v.run("load(payload,'privacy check')");
 const secretHtml=['insight-list','lineage','environment','security','status'].map(id=>v.nodes.get(id).textContent).join('\n');
 assert.doesNotMatch(secretHtml,/super-secret-value/);
 assert.match(v.nodes.get('insight-list').textContent,/redacted/);
 assert.match(v.nodes.get('security').textContent,/sandbox_policy/);
 assert.match(v.nodes.get('security').textContent,/workspace-write/);
 assert.doesNotMatch(v.nodes.get('lineage').textContent,/super-secret-value/);
});

const EXPECTED_INSIGHT_IDS = Array.from({length:25},(_,i)=>`I${String(i+1).padStart(2,'0')}`);
const L0 = 'status, finding, practical meaning, scope, and missingness';
const L1 = 'assumptions and alternatives';
const L2 = 'safe evidence IDs and source versions';
const L3 = 'formula, population (what, included, excluded), exclusions, and cutoff';

function markerBlock(text, file) {
 const start = text.indexOf('@vnext-insight-catalog');
 const end = text.indexOf('@vnext-insight-catalog-end');
 assert.ok(start >= 0 && end > start, `catalog markers missing in ${file}`);
 return text.slice(start, end);
}
function insightIds(text, file) {
 const ids = [...markerBlock(text, file).matchAll(/\bI(?:0[1-9]|1\d|2[0-5])\b/g)].map(match => match[0]);
 return [...new Set(ids)].sort();
}
function viewNames(text) {
 return [...new Set([...text.matchAll(/\bv_[a-z0-9_]+/g)].map(match => match[0]))].sort();
}
function displayStatusFrom(source, file) {
 const start = source.indexOf('function display_status');
 const end = source.indexOf('\n}', start);
 assert.ok(start >= 0 && end > start, `display_status missing in ${file}`);
 const context = vm.createContext({});
 vm.runInContext(source.slice(start, end + 2), context);
 return (status, reason) => vm.runInContext(`display_status(${JSON.stringify(status)}, ${JSON.stringify(reason)})`, context);
}

test('vNext insight ids, Nao views, obsidian tokens, and exclusive badges', () => {
 const root = path.join(__dirname, '..');
 const readRoot = rel => fs.readFileSync(path.join(root, rel), 'utf8');
 const catalogs = {
  registry: insightIds(readRoot('crates/dendro-core/src/insight_v1/reference/mod.rs'), 'reference'),
  presentation: insightIds(readRoot('crates/dendro-core/src/presentation.rs'), 'presentation'),
  viewer: insightIds(read('viewer.html'), 'viewer'),
  index: insightIds(read('dashboards/index.html'), 'index'),
  tui: insightIds(readRoot('crates/dendro-cli/src/tui/mod.rs'), 'tui'),
 };
 for (const [name, ids] of Object.entries(catalogs)) {
  assert.deepEqual(ids, EXPECTED_INSIGHT_IDS, name);
 }
 const viewsRs = readRoot('crates/dendro-cli/src/tui/views.rs');
 const tuiMod = readRoot('crates/dendro-cli/src/tui/mod.rs');
 assert.match(viewsRs, /VNEXT_INSIGHT_INDEX/);
 [L0, L1, L2, L3].forEach(label => {
  assert.ok(viewsRs.includes(label), label);
  assert.ok(tuiMod.includes(label) || viewsRs.includes(label), label);
 });
 assert.ok(tuiMod.includes(L3));

 const indexHtml = read('dashboards/index.html');
 assert.equal((indexHtml.match(/data-persona-question="/g) || []).length, 7);
 assert.equal((indexHtml.match(/data-insight-id="I/g) || []).length, 25);
 assert.match(indexHtml, /href="views\/trajectory_vortex.html"[^>]*data-category="harness"/);
 assert.match(indexHtml, /href="views\/schema_drift.html"[^>]*data-category="harness"/);
 assert.match(indexHtml, /href="views\/authority_blast_radius.html"[^>]*data-category="environment"/);
 assert.match(indexHtml, /href="views\/counterfactual_routing.html"[^>]*data-category="model"/);
 assert.match(indexHtml, /Compiled Insights Strip \(insight_v1\)/);

 const viewFiles = [
  'crates/dendro-core/src/semantic_views.sql',
  'crates/dendro-core/src/semantic.rs',
  'templates/nao/semantic_models.yaml',
  'templates/nao/nao_config.yaml',
  'templates/nao/prompts.md',
  'docs/NAO_SEMANTIC.md',
 ];
 const expectedViews = viewNames(readRoot(viewFiles[0]));
 assert.equal(expectedViews.length, 12);
 viewFiles.forEach(rel => assert.deepEqual(viewNames(readRoot(rel)), expectedViews, rel));
 const yamlViews = new Set();
 for (const file of fs.readdirSync(path.join(root, 'templates/nao/dashboards'))) {
  if (!file.endsWith('.yaml')) continue;
  viewNames(fs.readFileSync(path.join(root, 'templates/nao/dashboards', file), 'utf8')).forEach(name => yamlViews.add(name));
 }
 assert.deepEqual([...yamlViews].sort(), expectedViews);
 const htmlViews = new Set();
 for (const file of fs.readdirSync(path.join(__dirname, 'dashboards/views'))) {
  if (!file.endsWith('.html')) continue;
  const html = read(`dashboards/views/${file}`);
  assert.doesNotMatch(html, /<(?:script|link|img|iframe)[^>]+(?:src|href)="https?:/i);
  assert.match(html, /id="live-duckdb-panel"/);
  assert.match(html, /id="live-duckdb-window"/);
  assert.match(html, /id="live-duckdb-facts"/);
  assert.match(html, /id="live-duckdb-body"/);
  assert.match(html, /attachLiveSync/);
  viewNames(html).forEach(name => htmlViews.add(name));
 }
 assert.deepEqual([...htmlViews].sort(), expectedViews);
 ['trajectory_vortex', 'schema_drift', 'authority_blast_radius', 'counterfactual_routing'].forEach(slug => {
  const html = read(`dashboards/views/${slug}.html`);
  assert.match(html, /All-time counts — window_from\/window_to label the snapshot but do not filter\./);
  assert.match(html, /ILLUSTRATIVE SNAPSHOT/);
  assert.doesNotMatch(html, /LIVE DUCKDB/);
 });
 assert.match(read('dashboards/views/trajectory_vortex.html'), /not a cause/);
 assert.match(read('dashboards/views/schema_drift.html'), /insufficient_sample/);
 assert.match(read('dashboards/views/authority_blast_radius.html'), /dependency exposure/);
 assert.match(read('dashboards/views/counterfactual_routing.html'), /not an effect/);

 const css = read('dashboards/style.css');
 const viewerHtml = read('viewer.html');
 assert.match(css, /--bg-obsidian:\s*#0A0D12/);
 assert.match(css, /--bg-surface:\s*#11161F/);
 assert.match(viewerHtml, /--bg-obsidian:#0A0D12/);
 assert.match(viewerHtml, /--bg-surface:#11161F/);
 assert.doesNotMatch(css, /#090d16|#0f1927/i);
 assert.doesNotMatch(viewerHtml, /#090d16|#0f1927/i);
 const htmlFiles = ['viewer.html', 'dashboards/index.html', ...fs.readdirSync(path.join(__dirname, 'dashboards/views')).filter(name => name.endsWith('.html')).map(name => `dashboards/views/${name}`)];
 htmlFiles.forEach(rel => {
  const html = read(rel);
  assert.match(html, /ILLUSTRATIVE SNAPSHOT/);
  assert.doesNotMatch(html, /LIVE DUCKDB/);
  assert.doesNotMatch(html, /ILLUSTRATIVE SNAPSHOT · LIVE SYNC READY/);
  assert.doesNotMatch(html, /#090d16|#0f1927/i);
 });
 const live = read('dashboards/live-sync.js');
 assert.match(live, /SNAPSHOT_BADGE = 'ILLUSTRATIVE SNAPSHOT'/);
 assert.match(live, /LIVE_BADGE = 'LIVE DUCKDB'/);
 assert.match(live, /badgeText\.textContent = LIVE_BADGE/);
 assert.match(live, /badgeText\.textContent = SNAPSHOT_BADGE/);
 assert.doesNotMatch(live, /LIVE SYNC READY|LIVE SSE|POLLING DUCKDB|LIVE HTTP/);
 assert.match(live, /function renderLiveDuckDbPanel/);
 assert.match(live, /recorded_window/);
 assert.match(live, /table_counts/);
 assert.match(live, /live-duckdb-panel/);
 assert.match(live, /endpoint: '\/api\/events'/);
 assert.match(live, /pollEndpoint: '\/api\/query'/);
 assert.match(indexHtml, /viewId: 'hub'/);
 assert.match(indexHtml, /id="hub-recorded-window"/);
 assert.match(indexHtml, /id="hub-layer-summaries"/);
 assert.match(indexHtml, /source === 'sse' \|\| source === 'poll'/);
 assert.match(viewerHtml, /\/api\/query\?view=cockpit/);
 assert.match(viewerHtml, /LIVE '\+'DUCKDB'/);

 for (const rel of ['viewer.html', 'dashboards/index.html']) {
  const fn = displayStatusFrom(read(rel), rel);
  assert.equal(fn('Refused', 'not_observable'), 'insufficient_evidence');
  assert.equal(fn('Refused', 'insufficient_sample'), 'insufficient_evidence');
  assert.equal(fn('Computed', 'threshold_exceeded'), 'partial');
  assert.equal(fn('Refused', 'threshold_exceeded'), 'partial');
  assert.equal(fn('Computed', ''), 'ok');
  assert.equal(fn('Refused', ''), 'refused');
 }
});

test('live poll treats JSON null as not live, a non-null object as live, and a later failure as not live', { timeout: 5000 }, async () => {
 function makeClassList() {
  const names = new Set();
  return {
   add(...items) { for (const item of items) names.add(item); },
   remove(...items) { for (const item of items) names.delete(item); },
   contains(item) { return names.has(item); }
  };
 }
 const badge = { classList: makeClassList() };
 const badgeText = { textContent: '' };
 const calls = [];
 const context = vm.createContext({
  window: { location: { protocol: 'http:' } },
  document: {
   getElementById(id) {
    if (id === 'dual-serving-indicator') return badge;
    if (id === 'dual-serving-text') return badgeText;
    return null;
   }
  },
  EventSource: function EventSource() { this.close = function() {}; },
  setInterval() { return 1; },
  clearInterval() {},
  setTimeout,
  queueMicrotask,
  console,
  calls,
  badge
 });
 vm.runInContext(read('dashboards/live-sync.js'), context);
 const raw = await vm.runInContext(`(async () => {
  let payload = null;
  let fail = false;
  globalThis.fetch = function() {
   let markSettled;
   globalThis.__pollDone = new Promise(resolve => { markSettled = resolve; });
   if (fail) {
    const rejected = Promise.reject(new Error('poll failed'));
    rejected.catch(() => queueMicrotask(markSettled));
    return rejected;
   }
   return Promise.resolve({
    ok: true,
    json() {
     const parsed = Promise.resolve(payload);
     parsed.then(() => queueMicrotask(markSettled));
     return parsed;
    }
   });
  };
  async function awaitPoll() {
   await Promise.race([
    globalThis.__pollDone,
    new Promise((_, reject) => setTimeout(() => reject(new Error('poll did not settle')), 1000))
   ]);
  }
  async function exercise(next) {
   payload = next;
   fail = false;
   calls.length = 0;
   badge.classList.remove('live', 'snapshot');
   const handle = window.attachLiveSync({
    onData(data, source) { calls.push({ data: data, source: source }); }
   });
   handle.refresh();
   await awaitPoll();
   return {
    isLive: handle.isLive,
    live: badge.classList.contains('live'),
    poll: calls.filter(call => call.source === 'poll')
   };
  }
  const nullCase = await exercise(null);
  const liveCase = await exercise({ ok: true });
  payload = { ok: true };
  fail = false;
  calls.length = 0;
  badge.classList.remove('live', 'snapshot');
  const handle = window.attachLiveSync({
   onData(data, source) { calls.push({ data: data, source: source }); }
  });
  handle.refresh();
  await awaitPoll();
  const wasLive = handle.isLive;
  fail = true;
  handle.refresh();
  await awaitPoll();
  const failedAfterLive = {
   wasLive: wasLive,
   isLive: handle.isLive,
   live: badge.classList.contains('live')
  };
  return JSON.stringify({ nullCase, liveCase, failedAfterLive });
 })()`, context);
 const result = JSON.parse(raw);
 assert.equal(result.nullCase.isLive, false);
 assert.equal(result.nullCase.live, false);
 assert.deepEqual(result.nullCase.poll, []);
 assert.equal(result.liveCase.isLive, true);
 assert.equal(result.liveCase.live, true);
 assert.equal(result.liveCase.poll.length, 1);
 assert.deepEqual(result.liveCase.poll[0].data, { ok: true });
 assert.equal(result.liveCase.poll[0].source, 'poll');
 assert.equal(result.failedAfterLive.wasLive, true);
 assert.equal(result.failedAfterLive.isLive, false);
 assert.equal(result.failedAfterLive.live, false);
});
