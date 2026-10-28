'use strict';
// All values are operator assumptions. No network calls or hidden benchmark constants.
function modelScenario({seats,missions,tokensK,reworkPercent,costPerMillion,hours,reduction}) {
  const returned = missions * reworkPercent / 100;
  const tokenMillions = returned * tokensK / 1000;
  return {returned,tokenMillions,cost:tokenMillions*costPerMillion,
    hours:returned*hours*reduction/100,capacity:reworkPercent*reduction/100,
    missionsPerSeat:missions/seats};
}

document.addEventListener('DOMContentLoaded', () => {
  // 1. Capacity Scenario Calculator (Operator Assumption Explorer)
  const fields={seats:'seatsInput',missions:'missionsInput',tokensK:'tokensPerMissionInput',reworkPercent:'reworkRateInput',costPerMillion:'tokenCostInput',hours:'hoursInput',reduction:'reductionInput'};
  const format=n=>n.toLocaleString(undefined,{maximumFractionDigits:2});
  const write=(id,text)=>{const el=document.getElementById(id);if(el)el.textContent=text;};
  function update(){
    const input=Object.fromEntries(Object.entries(fields).map(([key,id])=>[key,Number(document.getElementById(id).value)]));
    const r=modelScenario(input);
    write('seatsVal',`${input.seats} actors`);write('missionsVal',`${input.missions} missions`);
    write('tokensPerMissionVal',`${input.tokensK}k tokens`);write('reworkRateVal',`${input.reworkPercent}%`);
    write('tokenCostVal',`$${input.costPerMillion} / 1M`);write('hoursVal',`${input.hours} hours`);write('reductionVal',`${input.reduction}%`);
    write('reworkChainsVal',`${format(r.returned)} expected returns`);write('reworkTokensVal',`${format(r.tokenMillions)}M tokens`);
    write('reworkCostVal',`$${r.cost.toLocaleString(undefined,{minimumFractionDigits:2,maximumFractionDigits:2})} USD`);
    write('hoursSavedVal',`${format(r.hours)} hours / month`);write('capacityVal',`${format(r.capacity)}%`);
    write('seatLoadVal',`${format(r.missionsPerSeat)} missions per actor / month, assuming an even workload.`);
  }
  if(Object.values(fields).every(id=>document.getElementById(id))){
    Object.values(fields).forEach(id=>document.getElementById(id).addEventListener('input',update));
    update();
  }

  // 2. Hero Multi-Screen Horizontal Screenshot Carousel
  const stageTabs = Array.from(document.querySelectorAll('[data-stage-tab]'));
  const stagePanels = Array.from(document.querySelectorAll('[data-stage-panel]'));
  const carouselTrack = document.getElementById('screenshot-carousel-track');
  const prevBtn = document.getElementById('carousel-prev');
  const nextBtn = document.getElementById('carousel-next');
  let activeStageIdx = 0;

  function setActiveStageState(targetKey, focusTab) {
    stageTabs.forEach((tab, idx) => {
      const active = tab.getAttribute('data-stage-tab') === targetKey;
      if (active) activeStageIdx = idx;
      tab.setAttribute('aria-selected', active ? 'true' : 'false');
      tab.setAttribute('tabindex', active ? '0' : '-1');
      tab.classList.toggle('is-active', active);
      if (active && focusTab) tab.focus();
    });
    stagePanels.forEach(panel => {
      const active = panel.getAttribute('data-stage-panel') === targetKey;
      panel.classList.toggle('is-active', active);
      panel.removeAttribute('hidden');
    });
  }

  function scrollToStageSlide(targetKey, focusTab) {
    setActiveStageState(targetKey, focusTab);
    const targetSlide = stagePanels.find(p => p.getAttribute('data-stage-panel') === targetKey);
    if (targetSlide && typeof targetSlide.scrollIntoView === 'function') {
      targetSlide.scrollIntoView({ behavior: 'smooth', block: 'nearest', inline: 'center' });
    }
  }

  stageTabs.forEach((tab, idx) => {
    tab.addEventListener('click', () => scrollToStageSlide(tab.getAttribute('data-stage-tab'), false));
    tab.addEventListener('keydown', e => {
      if (e.key === 'ArrowRight' || e.key === 'ArrowLeft') {
        e.preventDefault();
        const nextIdx = (idx + (e.key === 'ArrowRight' ? 1 : stageTabs.length - 1)) % stageTabs.length;
        scrollToStageSlide(stageTabs[nextIdx].getAttribute('data-stage-tab'), true);
      }
    });
  });

  if (prevBtn && stageTabs.length) {
    prevBtn.addEventListener('click', () => {
      const nextIdx = (activeStageIdx + stageTabs.length - 1) % stageTabs.length;
      scrollToStageSlide(stageTabs[nextIdx].getAttribute('data-stage-tab'), false);
    });
  }
  if (nextBtn && stageTabs.length) {
    nextBtn.addEventListener('click', () => {
      const nextIdx = (activeStageIdx + 1) % stageTabs.length;
      scrollToStageSlide(stageTabs[nextIdx].getAttribute('data-stage-tab'), false);
    });
  }
  if (carouselTrack && stagePanels.length) {
    let scrollTick = false;
    carouselTrack.addEventListener('scroll', () => {
      if (scrollTick) return;
      scrollTick = true;
      window.requestAnimationFrame(() => {
        const trackRect = carouselTrack.getBoundingClientRect();
        const trackCenter = trackRect.left + trackRect.width / 2;
        let closestKey = null;
        let minDist = Infinity;
        stagePanels.forEach(panel => {
          const rect = panel.getBoundingClientRect();
          const slideCenter = rect.left + rect.width / 2;
          const dist = Math.abs(slideCenter - trackCenter);
          if (dist < minDist) {
            minDist = dist;
            closestKey = panel.getAttribute('data-stage-panel');
          }
        });
        if (closestKey) setActiveStageState(closestKey, false);
        scrollTick = false;
      });
    }, { passive: true });
  }

  // 3. Living Growth Rings (#dendro-growth-rings) & Tactile Time Machine Scrubber
  const sectorNodes = Array.from(document.querySelectorAll('[data-ring-sector]'));
  function inspectSector(node) {
    if (!node) return;
    sectorNodes.forEach(s => s.classList.toggle('is-selected', s === node));
    write('heartwood-layer', node.getAttribute('data-layer-label') || 'All 4 Layers · Nominal');
    write('heartwood-slot', node.getAttribute('data-slot-window') || 'Oct 20 – Oct 27, 2026');
    write('heartwood-actor', node.getAttribute('data-actor-summary') || '5 Harnesses · 12 Actors');
    write('heartwood-metric', node.getAttribute('data-telemetry-metric') || '96.9% Cache Yield · 6.85B Tokens');
    write('heartwood-status', node.getAttribute('data-epistemic-status') || 'status: ok (computed)');
  }
  sectorNodes.forEach(node => {
    node.addEventListener('mouseenter', () => inspectSector(node));
    node.addEventListener('click', () => inspectSector(node));
    node.addEventListener('focus', () => inspectSector(node));
  });

  const timePresets = Array.from(document.querySelectorAll('[data-time-preset]'));
  timePresets.forEach(btn => {
    btn.addEventListener('click', () => {
      timePresets.forEach(b => {
        const active = b === btn;
        b.classList.toggle('is-active', active);
        b.setAttribute('aria-pressed', active ? 'true' : 'false');
      });
      write('scrubber-active-range', btn.getAttribute('data-range-label') || 'Oct 20 – Oct 27, 2026');
      write('heartwood-slot', btn.getAttribute('data-range-label') || 'Oct 20 – Oct 27, 2026');
      write('heartwood-metric', btn.getAttribute('data-range-metric') || '96.9% Cache Yield · 6.85B Tokens');
    });
  });

  // 4. 4-Layer Exploding Stack X-Ray (⌘E)
  const stackStage = document.getElementById('stack-xray-stage');
  const explodeButtons = Array.from(document.querySelectorAll('[data-stack-mode]'));
  function setStackMode(mode) {
    if (!stackStage) return;
    const isExploded = mode === '3d';
    stackStage.setAttribute('data-exploded', isExploded ? 'true' : 'false');
    explodeButtons.forEach(btn => {
      const active = btn.getAttribute('data-stack-mode') === mode;
      btn.classList.toggle('is-active', active);
      btn.setAttribute('aria-pressed', active ? 'true' : 'false');
    });
  }
  explodeButtons.forEach(btn => {
    btn.addEventListener('click', () => setStackMode(btn.getAttribute('data-stack-mode')));
  });
  document.addEventListener('keydown', e => {
    if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'e' && stackStage) {
      e.preventDefault();
      const next = stackStage.getAttribute('data-exploded') === 'true' ? '2d' : '3d';
      setStackMode(next);
    }
  });

  const stackPlanes = Array.from(document.querySelectorAll('[data-stack-layer]'));
  stackPlanes.forEach(plane => {
    plane.addEventListener('click', () => {
      stackPlanes.forEach(p => p.classList.toggle('is-focused', p === plane));
      write('xray-inspector-title', plane.getAttribute('data-layer-title') || '');
      write('xray-inspector-l0', plane.getAttribute('data-l0') || '');
      write('xray-inspector-l1', plane.getAttribute('data-l1') || '');
      write('xray-inspector-l2', plane.getAttribute('data-l2') || '');
      write('xray-inspector-l3', plane.getAttribute('data-l3') || '');
    });
  });

  // 5. Multi-Dimensional Group-By & Rollup Matrix
  const groupTabs = Array.from(document.querySelectorAll('[data-groupby-tab]'));
  const groupPanels = Array.from(document.querySelectorAll('[data-groupby-panel]'));
  groupTabs.forEach(tab => {
    tab.addEventListener('click', () => {
      const key = tab.getAttribute('data-groupby-tab');
      groupTabs.forEach(t => {
        const active = t === tab;
        t.classList.toggle('is-active', active);
        t.setAttribute('aria-selected', active ? 'true' : 'false');
      });
      groupPanels.forEach(p => {
        p.hidden = p.getAttribute('data-groupby-panel') !== key;
      });
    });
  });
});
