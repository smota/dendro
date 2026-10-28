/**
 * Dendro Live Sync Client Hook
 * Provides dual-serving support: connects to live SSE / polling endpoint when served over HTTP,
 * or gracefully defaults to static DuckDB snapshot mode when served offline or via file://.
 */
(function(global) {
  'use strict';

  function attachLiveSync(options) {
    const config = Object.assign({
      viewId: 'general',
      badgeId: 'dual-serving-indicator',
      badgeTextId: 'dual-serving-text',
      endpoint: '/api/events',
      pollEndpoint: '/api/query',
      pollInterval: 4000,
      embeddedDataId: 'dendro-data',
      onData: null
    }, options || {});

    let sse = null;
    let pollTimer = null;
    let retryTimer = null;
    let isLive = false;
    let sseUnavailable = false;
    let connectedWindow = null;
    let liveGeneration = 0;

    const badge = document.getElementById(config.badgeId);
    const badgeText = document.getElementById(config.badgeTextId);

    const SNAPSHOT_BADGE = 'ILLUSTRATIVE SNAPSHOT';
    const LIVE_BADGE = 'LIVE DUCKDB';
    var FACT_KEYS = [
      'usage_facts',
      'message_facts',
      'activity_facts',
      'queue_facts',
      'security_facts',
      'session_edges',
      'tool_facts',
      'skill_facts'
    ];

    function factCount(counts, key) {
      if (!counts || typeof counts !== 'object') return null;
      var value = counts[key];
      if (typeof value === 'number' && isFinite(value)) return value;
      if (value && typeof value === 'object' && typeof value.rows === 'number' && isFinite(value.rows)) return value.rows;
      return null;
    }

    function displayColumn(key) {
      switch (key) {
        case 'seat_id':
        case 'seat':
          return 'actor_id';
        case 'from_seat':
          return 'from_actor';
        case 'to_seat':
          return 'to_actor';
        case 'rig':
          return 'scope_id';
        case 'four_eyes':
        case 'four_eyes_status':
          return 'peer_review';
        default:
          return key;
      }
    }

    function formatCell(value) {
      if (value == null) return '—';
      if (typeof value === 'object') {
        try { return JSON.stringify(value); } catch (err) { return '—'; }
      }
      if (value === 'cross_rig_seam') return 'cross_scope_seam';
      if (typeof value === 'string') {
        var friendly = friendlyViewLabel(value);
        if (friendly) return friendly;
      }
      return String(value);
    }

    function hideLivePanel() {
      if (typeof document === 'undefined' || typeof document.getElementById !== 'function') return;
      var panel = document.getElementById('live-duckdb-panel');
      if (!panel) return;
      if (typeof panel.getAttribute === 'function' && panel.getAttribute('data-keep-visible') === 'true') return;
      if (typeof panel.closest === 'function' && panel.closest('.dendro-l3-drawer')) return;
      var drawerParent = panel.parentNode;
      if (drawerParent && drawerParent.classList && drawerParent.classList.contains('dendro-l3-drawer')) return;
      panel.hidden = true;
    }

    function ensureLivePanel() {
      var panel = document.getElementById('live-duckdb-panel');
      if (panel) return panel;
      if (typeof document.createElement !== 'function') return null;
      var mount = null;
      if (typeof document.querySelector === 'function') {
        mount = document.querySelector('main') || document.querySelector('.telemetry-layout');
      }
      if (!mount && document.body) mount = document.body;
      if (!mount || typeof mount.insertBefore !== 'function') return null;
      panel = document.createElement('section');
      panel.id = 'live-duckdb-panel';
      panel.className = 'live-duckdb-panel';
      panel.hidden = true;
      panel.setAttribute('aria-label', 'Live DuckDB window');
      var surface = document.createElement('div');
      surface.className = 'live-duckdb-surface';
      surface.style.background = '#11161F';
      var title = document.createElement('h2');
      title.className = 'live-duckdb-title';
      title.textContent = 'Recorded telemetry';
      var windowEl = document.createElement('p');
      windowEl.id = 'live-duckdb-window';
      var totalEl = document.createElement('p');
      totalEl.id = 'live-duckdb-totals';
      var facts = document.createElement('ul');
      facts.id = 'live-duckdb-facts';
      facts.className = 'fact-counts';
      var scroll = document.createElement('div');
      scroll.className = 'live-duckdb-scroll';
      var table = document.createElement('table');
      table.className = 'telemetry-table';
      var head = document.createElement('thead');
      head.id = 'live-duckdb-head';
      var body = document.createElement('tbody');
      body.id = 'live-duckdb-body';
      table.appendChild(head);
      table.appendChild(body);
      scroll.appendChild(table);
      surface.appendChild(title);
      surface.appendChild(windowEl);
      surface.appendChild(totalEl);
      surface.appendChild(facts);
      surface.appendChild(scroll);
      panel.appendChild(surface);
      panel.style.background = '#0A0D12';
      mount.insertBefore(panel, mount.firstChild || null);
      return panel;
    }

    function renderLiveDuckDbPanel(data, source) {
      if (source !== 'sse' && source !== 'poll') return;
      if (!data || typeof data !== 'object') return;
      var hasRows = Array.isArray(data.rows) && data.rows.length > 0;
      var hasCounts = !!data.table_counts && typeof data.table_counts === 'object';
      if (!hasRows && !hasCounts) return;
      if (typeof document === 'undefined' || typeof document.getElementById !== 'function' || typeof document.createElement !== 'function') return;
      var panel = ensureLivePanel();
      if (!panel) return;
      panel.hidden = false;

      var recorded = data.recorded_window || {};
      var from = recorded.from == null || recorded.from === '' ? '—' : String(recorded.from);
      var to = recorded.to == null || recorded.to === '' ? '—' : String(recorded.to);
      var windowEl = document.getElementById('live-duckdb-window');
      if (windowEl) {
        var humanWindow = recorded.display || formatHumanWindow(recorded.from_date || recorded.from, recorded.to_date || recorded.to, getPersistedWindow());
        windowEl.textContent = 'Recorded window ' + humanWindow;
        if (typeof windowEl.setAttribute === 'function' && (from !== '—' || to !== '—')) {
          windowEl.setAttribute('title', from + ' → ' + to);
        }
      }

      var rows = Array.isArray(data.rows) ? data.rows : [];
      var total = typeof data.row_count === 'number' ? data.row_count : rows.length;
      var totalEl = document.getElementById('live-duckdb-totals');
      if (totalEl) totalEl.textContent = 'Total rows: ' + total.toLocaleString();
      var countBadge = document.getElementById('record-count-badge');
      if (countBadge) {
        var viewName = (data && typeof data.view === 'string' && data.view) || config.viewId || '';
        countBadge.replaceChildren();
        if (typeof viewName === 'string' && viewName.indexOf('v_') === 0) {
          countBadge.appendChild(document.createTextNode(rows.length + ' recorded rows'));
          var hiddenView = document.createElement('span');
          hiddenView.className = 'sr-only';
          hiddenView.textContent = viewName;
          countBadge.appendChild(hiddenView);
        } else {
          countBadge.textContent = rows.length + (rows.length === 1 ? ' Row' : ' Rows');
        }
      }

      var factsEl = document.getElementById('live-duckdb-facts');
      if (factsEl) {
        factsEl.replaceChildren();
        var coverageGroups = [
          { id: 'model', label: 'Model', keys: [['usage_facts', 'usage observations'], ['message_facts', 'message facts']] },
          { id: 'harness', label: 'Harness', keys: [['tool_facts', 'tool facts'], ['skill_facts', 'skill facts']] },
          { id: 'coord', label: 'Coordination', keys: [['activity_facts', 'activity intervals'], ['queue_facts', 'queue items'], ['session_edges', 'session lineage edges']] },
          { id: 'env', label: 'Environment', keys: [['security_facts', 'security posture records']] }
        ];
        coverageGroups.forEach(function (group) {
          var li = document.createElement('li');
          li.className = 'layer-coverage-badge';
          li.setAttribute('data-layer', group.id);
          var parts = group.keys.map(function (pair) {
            var count = factCount(data.table_counts, pair[0]);
            return pair[1] + ' ' + (count == null ? '—' : count.toLocaleString());
          });
          li.textContent = group.label + ' · ' + parts.join(' · ');
          factsEl.append(li);
        });
      }

      var columns = [];
      var seen = {};
      rows.forEach(function (row) {
        if (!row || typeof row !== 'object' || Array.isArray(row)) return;
        Object.keys(row).forEach(function (key) {
          if (seen[key]) return;
          if (/^v_[a-z0-9_]+$/i.test(key)) return;
          seen[key] = true;
          columns.push(key);
        });
      });

      var head = document.getElementById('live-duckdb-head');
      var body = document.getElementById('live-duckdb-body');
      if (!head || !body) return;
      head.replaceChildren();
      body.replaceChildren();
      var headRow = document.createElement('tr');
      if (!columns.length) {
        var emptyHead = document.createElement('th');
        emptyHead.textContent = 'Value';
        headRow.append(emptyHead);
      } else {
        columns.forEach(function (key) {
          var th = document.createElement('th');
          th.textContent = displayColumn(key);
          headRow.append(th);
        });
      }
      head.append(headRow);

      if (!rows.length) {
        var emptyRow = document.createElement('tr');
        var emptyCell = document.createElement('td');
        emptyCell.colSpan = 1;
        emptyCell.textContent = 'No rows in this live result.';
        emptyRow.append(emptyCell);
        body.append(emptyRow);
        return;
      }

      var fragment = typeof document.createDocumentFragment === 'function' ? document.createDocumentFragment() : null;
      rows.forEach(function (row) {
        var tr = document.createElement('tr');
        if (!columns.length) {
          var only = document.createElement('td');
          only.textContent = formatCell(row);
          tr.append(only);
        } else {
          columns.forEach(function (key) {
            var td = document.createElement('td');
            var value = row && typeof row === 'object' ? row[key] : undefined;
            td.textContent = formatCell(value);
            tr.append(td);
          });
        }
        if (fragment) fragment.append(tr);
        else body.append(tr);
      });
      if (fragment) body.append(fragment);
    }

    function renderLivePanel(data, source) {
      return renderLiveDuckDbPanel(data, source);
    }

    function updateBadge(mode) {
      if (!badge) return;
      badge.classList.remove('live', 'snapshot');
      if (mode === 'live') {
        badge.classList.add('live');
        if (badgeText) badgeText.textContent = LIVE_BADGE;
        if (badgeText && typeof badgeText.setAttribute === 'function') {
          badgeText.setAttribute('data-display-label', 'LIVE TELEMETRY');
        }
      } else {
        badge.classList.add('snapshot');
        if (badgeText) badgeText.textContent = SNAPSHOT_BADGE;
        if (badgeText && typeof badgeText.removeAttribute === 'function') {
          badgeText.removeAttribute('data-display-label');
        }
      }
    }

    function loadEmbeddedData() {
      const el = document.getElementById(config.embeddedDataId);
      if (!el) return null;
      try {
        const text = el.textContent.trim();
        if (text && text !== 'null') {
          return JSON.parse(text);
        }
      } catch (err) {
        console.warn('Dendro: Failed to parse embedded snapshot JSON', err);
      }
      return null;
    }

    function triggerData(data, source) {
      var embedded = null;
      try { embedded = loadEmbeddedData(); } catch (err) { embedded = null; }
      var normalized = data;
      try { normalized = normalizeViewPayload(data, embedded, config.viewId); }
      catch (err) { normalized = data; }
      if (source === 'sse' || source === 'poll') {
        try { renderLivePanel(normalized, source); }
        catch (err) { console.error('Dendro: live DuckDB panel failed', err); }
      } else hideLivePanel();
      try {
        if (global.DendroState && typeof global.DendroState.set === 'function') {
          global.DendroState.set({
            lastPayload: normalized,
            live: source === 'sse' || source === 'poll'
          });
        }
      } catch (err) {}
      if (typeof config.onData === 'function') {
        try {
          config.onData(normalized, source);
        } catch (e) {
          console.error('Dendro: Error in onData callback:', e);
        }
      }
      try { paintActiveSlice(getPersistedWindow(), normalized); } catch (err) {}
      try { refreshDimensionPivot(normalized, config.viewId); } catch (err) {}
    }

    function publishLive(data) {
      if (data == null) return;
      isLive = true;
      updateBadge('live');
      triggerData(data, 'poll');
    }

    const initialWindow = getPersistedWindow();
    dendroState.preset = initialWindow;

    try {
      nestLivePanelInDrawer();
      mountTimeMachineScrubber(null, {
        viewId: config.viewId,
        pollEndpoint: config.pollEndpoint,
        publishLive: publishLive,
        reconnectForWindow: reconnectForWindow,
        emitLocal: function (localPayload) {
          if (typeof config.onData === 'function') {
            try { config.onData(localPayload, 'time-machine'); }
            catch (err) { console.error('Dendro: Error in onData callback:', err); }
          }
          try { paintActiveSlice(getPersistedWindow(), localPayload); } catch (err) {}
          try { refreshDimensionPivot(localPayload, config.viewId); } catch (err) {}
        },
        loadEmbedded: loadEmbeddedData
      });
      mountDimensionPivotBar(config.viewId);
    } catch (err) {}
    try { setPersistedWindow(initialWindow); } catch (err) {}

    // Initial check: if running offline / file protocol, immediately use snapshot mode
    const isHttp = window.location.protocol.startsWith('http');

    if (!isHttp) {
      updateBadge('snapshot');
      const data = loadEmbeddedData();
      if (data) triggerData(data, 'embedded-snapshot');
      return {
        isLive: false,
        mode: 'file-snapshot',
        disconnect: () => {},
        refresh: () => {
          const d = loadEmbeddedData();
          if (d) triggerData(d, 'embedded-snapshot');
        },
        reconnectForWindow: reconnectForWindow
      };
    }

    function getEventsUrl() {
      return `${config.endpoint}?view=${encodeURIComponent(config.viewId)}&window=${encodeURIComponent(getPersistedWindow(global))}`;
    }

    function getQueryUrl() {
      return `${config.pollEndpoint}?view=${encodeURIComponent(config.viewId)}&window=${encodeURIComponent(getPersistedWindow(global))}`;
    }

    function closeEventSource() {
      if (!sse) return;
      try { sse.onopen = null; sse.onmessage = null; sse.onerror = null; } catch (err) {}
      try { sse.close(); } catch (err) {}
      sse = null;
    }

    function clearRetryTimer() {
      if (!retryTimer) return;
      try { if (typeof clearTimeout === 'function') clearTimeout(retryTimer); } catch (err) {}
      retryTimer = null;
    }

    function reconnectForWindow(newPreset) {
      clearRetryTimer();
      if (newPreset && newPreset === connectedWindow && (sse || pollTimer)) return;
      if (newPreset) {
        try { setPersistedWindow(newPreset); } catch (err) {}
      }
      liveGeneration += 1;
      closeEventSource();
      if (!isHttp) return;
      if (sseUnavailable || typeof EventSource !== 'function') {
        pollOnce();
        return;
      }
      if (pollTimer) {
        clearInterval(pollTimer);
        pollTimer = null;
      }
      trySSE();
    }

    function onWindowPreset(event) {
      var detail = event && event.detail;
      var next = detail && detail.patch && detail.patch.preset;
      if (!next || next === connectedWindow) return;
      reconnectForWindow(next);
    }

    // If HTTP, attempt SSE first, then fall back to polling
    function trySSE() {
      var generation = liveGeneration;
      try {
        if (typeof EventSource !== 'function') {
          sseUnavailable = true;
          startPolling();
          return;
        }
        connectedWindow = getPersistedWindow(global);
        sse = new EventSource(getEventsUrl());

        sse.onopen = function() {
          if (generation !== liveGeneration) return;
          // The socket is up. The page is still showing the static snapshot
          // until a live payload hydrates, so the badge stays ILLUSTRATIVE SNAPSHOT.
        };

        sse.onmessage = function(event) {
          if (generation !== liveGeneration) return;
          try {
            const data = JSON.parse(event.data);
            if (data == null) return;
            isLive = true;
            updateBadge('live');
            triggerData(data, 'sse');
          } catch (err) {
            console.warn('Dendro: SSE payload parse failed', err);
          }
        };

        sse.onerror = function() {
          if (generation !== liveGeneration) return;
          closeEventSource();
          sseUnavailable = true;
          isLive = false;
          // Fall back to polling or snapshot
          startPolling();
        };
      } catch (err) {
        sseUnavailable = true;
        startPolling();
      }
    }

    function startPolling() {
      updateBadge('snapshot');
      pollOnce();
      if (!pollTimer && config.pollInterval > 0) {
        pollTimer = setInterval(pollOnce, config.pollInterval);
      }
    }

    async function pollOnce() {
      var generation = liveGeneration;
      connectedWindow = getPersistedWindow(global);
      try {
        const res = await fetch(getQueryUrl(), { cache: 'no-store' });
        if (generation !== liveGeneration) return;
        if (!res.ok) throw new Error(`HTTP ${res.status}`);
        const data = await res.json();
        if (generation !== liveGeneration) return;
        if (data !== null && data !== undefined) {
          isLive = true;
          updateBadge('live');
          triggerData(data, 'poll');
        }
      } catch (err) {
        if (generation !== liveGeneration) return;
        // Fall back to embedded data
        isLive = false;
        updateBadge('snapshot');
        const data = loadEmbeddedData();
        if (data) triggerData(data, 'embedded-snapshot');
      }
    }

    if (typeof window !== 'undefined' && typeof window.addEventListener === 'function') {
      window.addEventListener('dendro:state', onWindowPreset);
    }

    // Start connection
    trySSE();

    // Trigger initial embedded data if available right away
    const initialData = loadEmbeddedData();
    if (initialData) triggerData(initialData, 'initial-cache');

    return {
      get isLive() { return isLive; },
      disconnect: function() {
        liveGeneration += 1;
        if (typeof window !== 'undefined' && typeof window.removeEventListener === 'function') {
          window.removeEventListener('dendro:state', onWindowPreset);
        }
        clearRetryTimer();
        closeEventSource();
        if (pollTimer) { clearInterval(pollTimer); pollTimer = null; }
      },
      refresh: function() {
        if (isHttp) pollOnce();
        else {
          const d = loadEmbeddedData();
          if (d) triggerData(d, 'manual-refresh');
        }
      },
      reconnectForWindow: reconnectForWindow
    };
  }

  function num(value) {
    if (typeof value === 'number' && isFinite(value)) return value;
    if (typeof value === 'string' && value.trim() !== '' && isFinite(Number(value))) return Number(value);
    return 0;
  }

  function friendlyViewLabel(value) {
    if (typeof value !== 'string' || !/^v_[a-z0-9_]+$/i.test(value)) return null;
    var known = {
      v_seat_idle_ranking: 'Actor activity',
      v_handoff_rework_analysis: 'Handoff flow',
      v_mission_token_burn: 'Token burn',
      v_executive_governance: 'Governance scorecard',
      v_message_mix: 'Message mix',
      v_subagent_lineage: 'Subagent lineage',
      v_security_posture: 'Security posture',
      v_process_resources: 'Process resources',
      v_trajectory_vortex: 'Trajectory vortex',
      v_schema_drift: 'Schema drift',
      v_authority_blast_radius: 'Authority blast radius',
      v_counterfactual_routing: 'Counterfactual routing'
    };
    return known[value] || 'Telemetry view';
  }

  function canonicalConsole(name) {
    switch (name) {
      case 'seat_dynamics':
      case 'v_seat_idle_ranking':
        return 'seat_dynamics';
      case 'handoff_flow':
      case 'v_handoff_rework_analysis':
        return 'handoff_flow';
      case 'cost_and_loops':
      case 'v_mission_token_burn':
        return 'cost_and_loops';
      case 'executive_governance':
      case 'v_executive_governance':
        return 'executive_governance';
      default:
        return '';
    }
  }

  function missingList(value) {
    return !Array.isArray(value) || value.length === 0;
  }

  function fillSummary(target, built) {
    if (!built) return;
    if (!target.summary || typeof target.summary !== 'object' || Array.isArray(target.summary)) {
      target.summary = built;
      return;
    }
    Object.keys(built).forEach(function (key) {
      if (target.summary[key] == null) target.summary[key] = built[key];
    });
  }

  function synthesizeSeats(rows) {
    if (!Array.isArray(rows) || !rows.length) return [];
    rows = rows.filter(function (row) {
      return row && typeof row === 'object' && (row.seat_id || row.actor_id || row.seat || row.state || row.observed_secs != null || row.active_secs != null);
    });
    if (!rows.length) return [];
    var rich = rows.some(function (row) {
      return row && typeof row === 'object' && (row.observed_secs != null || row.active_secs != null || row.idle_with_work_secs != null);
    });
    if (rich) {
      return rows.filter(function (row) { return row && typeof row === 'object'; }).map(function (row) {
        return {
          seat_id: row.seat_id || row.actor_id || row.seat || 'actor',
          observed_secs: num(row.observed_secs),
          active_secs: num(row.active_secs),
          idle_with_work_secs: num(row.idle_with_work_secs),
          idle_between_work_secs: num(row.idle_between_work_secs),
          unparked_container_secs: num(row.unparked_container_secs),
          leaf_stall_secs: num(row.leaf_stall_secs),
          ceremony_ratio: num(row.ceremony_ratio),
          m1_yield: num(row.m1_yield),
          closed_items: row.closed_items
        };
      });
    }
    var byId = {};
    rows.forEach(function (row) {
      if (!row || typeof row !== 'object') return;
      var id = row.seat_id || row.actor_id || row.seat || 'actor';
      if (!byId[id]) {
        byId[id] = {
          seat_id: id,
          observed_secs: 0,
          active_secs: 0,
          idle_with_work_secs: 0,
          idle_between_work_secs: 0,
          unparked_container_secs: 0,
          leaf_stall_secs: 0,
          ceremony_ratio: 0,
          m1_yield: 0
        };
      }
      var seat = byId[id];
      var amount = num(row.count) || num(row.secs) || num(row.duration_secs) || 1;
      var state = String(row.state || row.activity_state || '').toLowerCase();
      seat.observed_secs += amount;
      if (state.indexOf('stall') !== -1 || state.indexOf('leaf') !== -1) seat.leaf_stall_secs += amount;
      else if (state.indexOf('unpark') !== -1 || state.indexOf('hold') !== -1 || state.indexOf('container') !== -1) seat.unparked_container_secs += amount;
      else if (state.indexOf('work') !== -1) seat.idle_with_work_secs += amount;
      else if (state.indexOf('idle') !== -1 || state.indexOf('between') !== -1 || state.indexOf('wait') !== -1) seat.idle_between_work_secs += amount;
      else seat.active_secs += amount;
      if (row.m1_yield != null) seat.m1_yield = num(row.m1_yield);
      if (row.ceremony_ratio != null) seat.ceremony_ratio = num(row.ceremony_ratio);
    });
    return Object.keys(byId).map(function (key) { return byId[key]; });
  }

  function summarizeSeats(seats) {
    var observed = 0, active = 0, idleWork = 0, holds = 0, ceremony = 0, yieldSum = 0;
    seats.forEach(function (seat) {
      observed += num(seat.observed_secs);
      active += num(seat.active_secs);
      idleWork += num(seat.idle_with_work_secs);
      holds += num(seat.unparked_container_secs);
      ceremony += num(seat.ceremony_ratio);
      yieldSum += num(seat.m1_yield);
    });
    var count = seats.length || 1;
    return {
      active_pct: observed ? (active / observed) * 100 : 0,
      idle_work_pct: observed ? (idleWork / observed) * 100 : 0,
      unparked_holds_sec: holds,
      avg_ceremony_ratio: ceremony / count,
      fleet_m1_yield: yieldSum / count
    };
  }

  function synthesizePairs(rows) {
    if (!Array.isArray(rows) || !rows.length) return [];
    rows = rows.filter(function (row) {
      return row && typeof row === 'object' && (row.from_seat || row.from_actor || row.source_seat || row.to_seat || row.to_actor || row.handoffs != null);
    });
    if (!rows.length) return [];
    return rows.map(function (row) {
      var handoffs = num(row.handoffs) || num(row.count) || 1;
      var rework = num(row.rework_returns);
      return {
        from_seat: row.from_seat || row.from_actor || row.source_seat || row.from || 'from',
        to_seat: row.to_seat || row.to_actor || row.dest_seat || row.to || 'to',
        handoffs: handoffs,
        rework_returns: rework,
        return_rate: row.return_rate != null ? num(row.return_rate) : (handoffs ? rework / handoffs : 0),
        wait_median_ms: num(row.wait_median_ms),
        exec_turnaround_median_ms: num(row.exec_turnaround_median_ms),
        ring_boundary: row.ring_boundary || 'in_ring'
      };
    });
  }

  function summarizePairs(pairs) {
    var handoffs = 0, rework = 0, boundaries = 0, wait = 0, exec = 0, timed = 0;
    pairs.forEach(function (pair) {
      handoffs += num(pair.handoffs);
      rework += num(pair.rework_returns);
      if (pair.ring_boundary && pair.ring_boundary !== 'in_ring') boundaries += 1;
      if (pair.wait_median_ms) { wait += num(pair.wait_median_ms); timed += 1; }
      exec += num(pair.exec_turnaround_median_ms);
    });
    return {
      total_handoffs: handoffs,
      fleet_return_rate: handoffs ? rework / handoffs : 0,
      rework_returns: rework,
      wait_median_secs: timed ? (wait / timed) / 1000 : 0,
      exec_median_secs: timed ? (exec / Math.max(timed, 1)) / 1000 : 0,
      boundary_crossings: boundaries
    };
  }

  function hasTokenAccounting(row) {
    return !!(row && typeof row === 'object' && (
      row.total_tokens != null || row.input_tokens != null || row.input_uncached != null ||
      row.cache_read_tokens != null || row.cache_read != null ||
      row.output_tokens != null || row.output != null
    ));
  }

  function observedNum(row, keys) {
    for (var i = 0; i < keys.length; i++) {
      if (row[keys[i]] != null) {
        var n = Number(row[keys[i]]);
        return isFinite(n) ? n : 0;
      }
    }
    return null;
  }

  function synthesizeMissions(rows) {
    if (!Array.isArray(rows) || !rows.length) return [];
    rows = rows.filter(hasTokenAccounting);
    if (!rows.length) return [];
    return rows.map(function (row) {
      var input = observedNum(row, ['input_uncached', 'fresh_input_tokens', 'input_tokens']);
      var cacheRead = observedNum(row, ['cache_read', 'cache_read_tokens']);
      var cacheWrite = observedNum(row, ['cache_write', 'cache_write_tokens']);
      var output = observedNum(row, ['output', 'output_tokens']);
      var observedParts = [input, cacheRead, cacheWrite, output].filter(function (value) { return value != null; });
      var total = row.total_tokens != null
        ? num(row.total_tokens)
        : (observedParts.length ? observedParts.reduce(function (sum, value) { return sum + value; }, 0) : null);
      var id = row.mission_id || row.session_id || row.seat_id || row.actor_id || row.harness || row.model || 'task';
      return {
        seat_id: row.seat_id || row.actor_id || row.harness || id,
        model: row.model || null,
        harness: row.harness || null,
        effort: row.effort || null,
        attribution_bucket: row.attribution_bucket || 'assigned_mission',
        mission_id: row.mission_id || row.session_id || id,
        session_id: row.session_id || row.mission_id || id,
        missions: Array.isArray(row.missions) ? row.missions : (row.mission_id ? [String(row.mission_id)] : []),
        input_uncached: input,
        cache_read: cacheRead,
        cache_write: cacheWrite,
        output: output,
        total_tokens: total,
        input_tokens: input,
        cache_read_tokens: cacheRead,
        output_tokens: output
      };
    });
  }

  function summarizeMissions(missions) {
    var total = 0, fresh = 0, cacheRead = 0, cacheWrite = 0, output = 0, intake = 0;
    missions.forEach(function (mission) {
      total += num(mission.total_tokens);
      fresh += num(mission.input_uncached);
      cacheRead += num(mission.cache_read);
      cacheWrite += num(mission.cache_write);
      output += num(mission.output);
      if (mission.attribution_bucket === 'task_intake_overhead') intake += num(mission.total_tokens);
    });
    var inputDenom = fresh + cacheRead + cacheWrite;
    return {
      total_tokens: total,
      cache_read_ratio: inputDenom ? cacheRead / inputDenom : 0,
      fresh_input_tokens: fresh,
      output_tokens: output,
      task_intake_overhead_pct: total ? intake / total : 0
    };
  }

  function synthesizeRecords(rows) {
    if (!Array.isArray(rows) || !rows.length) return [];
    rows = rows.filter(function (row) {
      return row && typeof row === 'object' && (row.rig || row.scope_id || row.scope || row.m1_yield != null || row.vortex_loops != null || row.four_eyes_independence || row.four_eyes_status || row.definition_version);
    });
    if (!rows.length) return [];
    return rows.map(function (row) {
      var compliant = true;
      if (row.four_eyes_independence && typeof row.four_eyes_independence === 'object') {
        compliant = row.four_eyes_independence.compliant !== false;
      } else if (typeof row.four_eyes_status === 'string' && /fail|violat/i.test(row.four_eyes_status)) {
        compliant = false;
      }
      return {
        rig: row.rig || row.scope_id || row.scope || 'default-fleet',
        window_from: row.window_from || null,
        window_to: row.window_to || null,
        definition_version: row.definition_version || 'v1',
        m1_yield: num(row.m1_yield),
        four_eyes_independence: row.four_eyes_independence || { compliant: compliant },
        watchdog_recovery: row.watchdog_recovery || { total_resumes: num(row.total_resumes) },
        vortex_loops: num(row.vortex_loops),
        context_compactions: num(row.context_compactions),
        fresh: row.fresh !== false
      };
    });
  }

  function summarizeRecords(records) {
    var loops = 0, yieldSum = 0, compliant = true;
    records.forEach(function (record) {
      loops += num(record.vortex_loops);
      yieldSum += num(record.m1_yield);
      if (record.four_eyes_independence && record.four_eyes_independence.compliant === false) compliant = false;
    });
    return {
      four_eyes_status: compliant ? 'COMPLIANT' : 'REVIEW',
      watchdog_health: 'OBSERVED',
      vortex_loops: loops,
      m1_yield: records.length ? yieldSum / records.length : 0,
      cross_slice_cache_bleed: 0
    };
  }

  function normalizeViewPayload(payload, embeddedData, viewName) {
    if (payload == null || typeof payload !== 'object' || Array.isArray(payload)) return payload;
    var embedded = embeddedData && typeof embeddedData === 'object' && !Array.isArray(embeddedData) ? embeddedData : null;
    var next = payload;
    var cloned = false;
    function fork() {
      if (cloned) return next;
      next = Object.assign({}, embedded || {}, payload);
      cloned = true;
      return next;
    }
    if (embedded) fork();
    var view = canonicalConsole(viewName) || canonicalConsole(next.view) || canonicalConsole(payload.view);
    var sourceRows = Array.isArray(payload.rows) ? payload.rows : next.rows;
    if (view === 'seat_dynamics' && missingList(payload.seats)) {
      var seats = synthesizeSeats(sourceRows);
      var seatsFromEmbedded = false;
      if (!seats.length && embedded && Array.isArray(embedded.seats)) {
        seats = embedded.seats;
        seatsFromEmbedded = true;
      }
      if (seats.length) {
        fork();
        next.seats = seats;
        if (!seatsFromEmbedded) next.summary = Object.assign({}, next.summary || {}, summarizeSeats(seats));
        else fillSummary(next, summarizeSeats(seats));
      }
    } else if (view === 'handoff_flow' && missingList(payload.pairs)) {
      var pairs = synthesizePairs(sourceRows);
      var pairsFromEmbedded = false;
      if (!pairs.length && embedded && Array.isArray(embedded.pairs)) {
        pairs = embedded.pairs;
        pairsFromEmbedded = true;
      }
      if (pairs.length) {
        fork();
        next.pairs = pairs;
        if (!pairsFromEmbedded) next.summary = Object.assign({}, next.summary || {}, summarizePairs(pairs));
        else fillSummary(next, summarizePairs(pairs));
      }
    } else if (view === 'cost_and_loops' && missingList(payload.missions)) {
      var missions = synthesizeMissions(sourceRows);
      var missionsFromEmbedded = false;
      if (!missions.length && embedded && Array.isArray(embedded.missions)) {
        missions = embedded.missions;
        missionsFromEmbedded = true;
      }
      if (missions.length) {
        fork();
        next.missions = missions;
        if (!missionsFromEmbedded) next.summary = Object.assign({}, next.summary || {}, summarizeMissions(missions));
        else fillSummary(next, summarizeMissions(missions));
      }
    } else if (view === 'executive_governance' && missingList(payload.records)) {
      var records = synthesizeRecords(sourceRows);
      var recordsFromEmbedded = false;
      if (!records.length && embedded && Array.isArray(embedded.records)) {
        records = embedded.records;
        recordsFromEmbedded = true;
      }
      if (records.length) {
        fork();
        next.records = records;
        if (!recordsFromEmbedded) next.summary = Object.assign({}, next.summary || {}, summarizeRecords(records));
        else fillSummary(next, summarizeRecords(records));
      }
    }
    return next;
  }

  var dendroState = {
    preset: 'all',
    range: null,
    hover: null,
    layer: null,
    focus: null,
    live: false,
    lastPayload: null
  };
  var dendroListeners = [];
  var STATE_KEYS = ['preset', 'range', 'hover', 'layer', 'focus', 'live', 'lastPayload'];

  function dendroSnapshot() {
    return {
      preset: dendroState.preset,
      range: dendroState.range,
      hover: dendroState.hover,
      layer: dendroState.layer,
      focus: dendroState.focus,
      live: dendroState.live,
      lastPayload: dendroState.lastPayload
    };
  }

  var DendroState = {
    get: function () { return dendroSnapshot(); },
    set: function (patch) {
      try {
        if (!patch || typeof patch !== 'object') return dendroSnapshot();
        var changed = false;
        STATE_KEYS.forEach(function (key) {
          if (!Object.prototype.hasOwnProperty.call(patch, key)) return;
          if (dendroState[key] !== patch[key]) {
            dendroState[key] = patch[key];
            changed = true;
          }
        });
        if (!changed) return dendroSnapshot();
        if (Object.prototype.hasOwnProperty.call(patch, 'preset') && normalizePreset(dendroState.preset)) {
          try { setPersistedWindow(dendroState.preset); } catch (err) {}
        }
        var snap = dendroSnapshot();
        dendroListeners.slice().forEach(function (fn) {
          try { fn(snap); } catch (err) {}
        });
        try {
          if (typeof CustomEvent === 'function' && typeof window !== 'undefined' && typeof window.dispatchEvent === 'function') {
            window.dispatchEvent(new CustomEvent('dendro:state', {
              detail: Object.assign({}, snap, { patch: patch })
            }));
          }
        } catch (err) {}
        return snap;
      } catch (err) {
        return dendroSnapshot();
      }
    },
    subscribe: function (fn) {
      if (typeof fn !== 'function') return function () {};
      dendroListeners.push(fn);
      return function () {
        dendroListeners = dendroListeners.filter(function (item) { return item !== fn; });
      };
    }
  };
  global.DendroState = DendroState;

  function simulateSpring(stiffness, damping, mass) {
    var x = 0;
    var velocity = 0;
    var dt = 1 / 120;
    var steps = 180;
    var samples = [];
    for (var i = 0; i <= steps; i++) {
      samples.push(x);
      var force = -stiffness * (x - 1) - damping * velocity;
      velocity += (force / mass) * dt;
      x += velocity * dt;
    }
    var points = [];
    for (var j = 0; j < samples.length; j += 6) {
      var y = samples[j];
      if (y > 1.2) y = 1.2;
      if (y < 0) y = 0;
      points.push(y.toFixed(4) + ' ' + ((j / (samples.length - 1)) * 100).toFixed(2) + '%');
    }
    var last = samples[samples.length - 1];
    if (last > 1.2) last = 1.2;
    if (last < 0) last = 0;
    points.push(last.toFixed(4) + ' 100%');
    return 'linear(' + points.join(', ') + ')';
  }

  var HUMAN_MONTHS = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];

  function pad2(value) {
    var number = Number(value);
    return number < 10 ? '0' + number : String(number);
  }

  function ymdFromMs(ms) {
    var dt = new Date(ms);
    return dt.getUTCFullYear() + '-' + pad2(dt.getUTCMonth() + 1) + '-' + pad2(dt.getUTCDate());
  }

  function parseClock(value) {
    if (value == null || value === '' || value === '—') return null;
    var text = String(value).trim();
    var dateOnly = /^(\d{4})-(\d{2})-(\d{2})$/.exec(text);
    if (dateOnly) {
      var year = +dateOnly[1];
      var month = +dateOnly[2];
      var day = +dateOnly[3];
      if (month < 1 || month > 12 || day < 1 || day > 31) return null;
      return {
        y: year,
        m: month,
        d: day,
        hour: null,
        minute: 0,
        second: 0,
        millis: 0,
        dateOnly: true,
        ms: Date.UTC(year, month - 1, day)
      };
    }
    if (/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}/.test(text) && !/(?:[zZ]|[+-]\d{2}:?\d{2})$/.test(text)) text += 'Z';
    var ms = Date.parse(text);
    if (!isFinite(ms)) return null;
    var dt = new Date(ms);
    return {
      y: dt.getUTCFullYear(),
      m: dt.getUTCMonth() + 1,
      d: dt.getUTCDate(),
      hour: dt.getUTCHours(),
      minute: dt.getUTCMinutes(),
      second: dt.getUTCSeconds(),
      millis: dt.getUTCMilliseconds(),
      dateOnly: false,
      ms: ms
    };
  }

  function splitRange(value) {
    if (typeof value !== 'string') return null;
    var marker = value.indexOf('..');
    if (marker <= 0 || value.indexOf('..', marker + 2) !== -1) return null;
    var from = value.slice(0, marker).trim();
    var to = value.slice(marker + 2).trim();
    if (!parseClock(from) || !parseClock(to)) return null;
    return [from, to];
  }

  function formatMonthDay(parts) {
    return HUMAN_MONTHS[parts.m - 1] + ' ' + parts.d;
  }

  function formatDayYear(parts) {
    return formatMonthDay(parts) + ', ' + parts.y;
  }

  function formatSpan(from, to) {
    if (from.y === to.y && from.m === to.m && from.d === to.d) return formatDayYear(from);
    if (from.y === to.y) return formatMonthDay(from) + ' – ' + formatMonthDay(to) + ', ' + to.y;
    return formatDayYear(from) + ' – ' + formatDayYear(to);
  }

  function roundedHour(clock) {
    if (!clock || clock.dateOnly || clock.hour == null) return null;
    var seconds = clock.hour * 3600 + (clock.minute || 0) * 60 + (clock.second || 0) + (clock.millis || 0) / 1000;
    var nearest = Math.round(seconds / 3600);
    if (nearest < 0) nearest = 0;
    if (nearest > 24) nearest = 24;
    return nearest;
  }

  function isExactUtcMidnight(clock) {
    return !!(clock && !clock.dateOnly && clock.hour === 0 && clock.minute === 0 && clock.second === 0 && (clock.millis || 0) === 0);
  }

  function formatHumanWindow(fromIso, toIso, preset) {
    var from = parseClock(fromIso);
    var to = parseClock(toIso);
    if (!from && !to) return '—';
    if (!from) from = to;
    if (!to) to = from;
    // Half-open windows end at the next UTC midnight. Date-only values stay inclusive.
    if (isExactUtcMidnight(to) && to.ms > from.ms) {
      var inclusiveMs = (from.dateOnly || isExactUtcMidnight(from)) ? to.ms - 86400000 : to.ms - 1;
      if (inclusiveMs < from.ms) inclusiveMs = from.ms;
      var inclusive = parseClock(new Date(inclusiveMs).toISOString());
      if (inclusive) to = inclusive;
    }
    var sameDay = from.y === to.y && from.m === to.m && from.d === to.d;
    if (!sameDay) return formatSpan(from, to);
    var startHour = roundedHour(from);
    var endHour = roundedHour(to);
    if (startHour == null || endHour == null || startHour === endHour) return formatDayYear(from);
    var endLabel = endHour === 24 ? '00:00' : pad2(endHour) + ':00';
    var startLabel = startHour === 24 ? '00:00' : pad2(startHour) + ':00';
    return formatMonthDay(from) + ' · ' + startLabel + ' – ' + endLabel;
  }

  function latestPayload() {
    try { return DendroState.get().lastPayload; }
    catch (err) { return null; }
  }

  function windowFrom(preset, payload) {
    var active = payload && payload.active_window;
    var recorded = payload && payload.recorded_window;
    if (active && typeof active === 'object' && (active.from || active.to)) return active;
    if (recorded && typeof recorded === 'object' && (recorded.from || recorded.to)) return recorded;
    var range = splitRange(preset);
    if (range) return { from: range[0], to: range[1] };
    return null;
  }

  function domainMillis(payload) {
    var source = payload || latestPayload();
    var win = source && (source.recorded_window || source.active_window);
    var from = win && parseClock(win.from);
    var to = win && parseClock(win.to);
    if (from && to && to.ms > from.ms) return { start: from.ms, end: to.ms };
    var now = new Date();
    var end = Date.UTC(now.getUTCFullYear(), now.getUTCMonth(), now.getUTCDate() + 1);
    return { start: end - 30 * 86400000, end: end };
  }

  function rangeForPreset(preset) {
    if (PRESET_WINDOWS[preset]) return PRESET_WINDOWS[preset].slice();
    var parts = splitRange(preset);
    if (!parts) return [0, 1];
    var domain = domainMillis(null);
    var from = parseClock(parts[0]);
    var to = parseClock(parts[1]);
    var span = domain.end - domain.start;
    if (!from || !to || span <= 0) return [0, 1];
    var start = (from.ms - domain.start) / span;
    var end = ((to.ms || from.ms) - domain.start) / span;
    if (to.dateOnly) end = ((to.ms + 86400000) - domain.start) / span;
    if (start < 0) start = 0;
    if (end > 1) end = 1;
    if (start > 1) start = 1;
    if (end < 0) end = 0;
    if (end < start) { var swap = start; start = end; end = swap; }
    if (end - start < 0.02) end = Math.min(1, start + 0.02);
    return [start, end];
  }

  function datesFromBrush(start, end, payload) {
    var domain = domainMillis(payload);
    var span = domain.end - domain.start;
    var fromMs = domain.start + span * Math.max(0, Math.min(1, start));
    var toMs = domain.start + span * Math.max(0, Math.min(1, end));
    if (toMs < fromMs) { var flip = fromMs; fromMs = toMs; toMs = flip; }
    var toDate = new Date(toMs);
    if (toDate.getUTCHours() === 0 && toDate.getUTCMinutes() === 0 && toDate.getUTCSeconds() === 0 && toMs > fromMs) {
      toMs -= 86400000;
    }
    if (toMs < fromMs) toMs = fromMs;
    return [ymdFromMs(fromMs), ymdFromMs(toMs)];
  }

  var WINDOW_LABELS = {
    today: 'Today (24h)',
    '24h': 'Last 24h',
    '1h': 'Last Hour',
    '7d': 'Last Week (7d)',
    '30d': '30 Days',
    this_month: 'This Month',
    last_month: 'Last Month',
    all: 'All Time'
  };

  var PIVOT_GROUPS = [
    { id: 'harness', label: 'By Harness / Tool' },
    { id: 'capability', label: 'By Capability / Tool Family' },
    { id: 'date', label: 'By Date (Daily)' },
    { id: 'rig', label: 'By Workspace / Team' },
    { id: 'status', label: 'By Status / Outcome' },
    { id: 'raw', label: 'Raw Records (Granular)' }
  ];

  var CONSOLE_TBODY_IDS = [
    'view-tbody', 'burn-tbody', 'seat-tbody', 'handoff-tbody', 'mix-tbody',
    'lineage-tbody', 'security-tbody', 'resource-tbody', 'governance-tbody'
  ];

  var pivotState = {
    group: 'harness',
    filterKey: null,
    expandedKey: null,
    rawOpen: true,
    viewId: '',
    records: []
  };

  function normalizePreset(raw) {
    if (raw == null) return '';
    var value = String(raw).trim();
    if (!value) return '';
    if (PRESET_WINDOWS[value]) return value;
    if (splitRange(value)) return value;
    return '';
  }

  function normalizeWindowPreset(value) {
    return normalizePreset(value);
  }

  function getPersistedWindow() {
    try {
      var search = '';
      if (typeof location !== 'undefined' && location && typeof location.search === 'string') search = location.search;
      if (typeof URLSearchParams === 'function') {
        var fromUrl = normalizeWindowPreset(new URLSearchParams(search).get('window'));
        if (fromUrl) return fromUrl;
      }
    } catch (err) {}
    try {
      if (typeof localStorage !== 'undefined' && localStorage && typeof localStorage.getItem === 'function') {
        var stored = normalizeWindowPreset(localStorage.getItem('dendro_time_window'));
        if (stored) return stored;
      }
    } catch (err) {}
    return 'all';
  }

  function activeSliceText(preset, payload) {
    var range = splitRange(preset);
    var label = range
      ? ('Custom (' + formatHumanWindow(range[0], range[1], preset) + ')')
      : (WINDOW_LABELS[preset] || WINDOW_LABELS.all);
    var text = '⏱ Active Slice: ' + label + ' · Applied Across All Views';
    var win = windowFrom(preset, payload);
    if (win && (win.display || win.from_date || win.to_date || win.from || win.to)) {
      text += ' · ' + (win.display || formatHumanWindow(win.from_date || win.from, win.to_date || win.to, preset));
    }
    return text;
  }

  function rewriteDashboardLinks(preset) {
    if (!preset || typeof document === 'undefined' || typeof document.querySelectorAll !== 'function') return;
    var anchors = document.querySelectorAll('a[href]');
    Array.prototype.forEach.call(anchors, function (anchor) {
      if (!anchor || typeof anchor.getAttribute !== 'function') return;
      var href = anchor.getAttribute('href');
      if (!href || href.charAt(0) === '#') return;
      if (/^(mailto|javascript|tel):/i.test(href)) return;
      var hashAt = href.indexOf('#');
      var hash = hashAt >= 0 ? href.slice(hashAt) : '';
      var before = hashAt >= 0 ? href.slice(0, hashAt) : href;
      if (/^[a-z][a-z0-9+.-]*:/i.test(before) && !/^https?:/i.test(before)) return;
      if (!/(\.html?)(?:$|\?)/i.test(before) && before.indexOf('/views/') === -1 && before.indexOf('dashboards') === -1) return;
      var path = before.split('?')[0];
      var query = before.indexOf('?') >= 0 ? before.slice(before.indexOf('?') + 1) : '';
      var params;
      try { params = new URLSearchParams(query); }
      catch (err) { return; }
      params.set('window', preset);
      var next = path + '?' + params.toString() + hash;
      if (href !== next) anchor.setAttribute('href', next);
    });
  }

  function highlightWindowButtons(preset) {
    if (typeof document === 'undefined' || typeof document.querySelectorAll !== 'function') return;
    var buttons = document.querySelectorAll('[data-window], [data-preset]');
    Array.prototype.forEach.call(buttons, function (btn) {
      if (!btn || typeof btn.getAttribute !== 'function') return;
      if (btn.closest && btn.closest('.dendro-pivot-bar')) return;
      var value = btn.getAttribute('data-window') || btn.getAttribute('data-preset');
      if (!value) return;
      var custom = !!splitRange(preset);
      var on = value === 'custom' ? custom : value === preset;
      if (value !== 'custom' && !PRESET_WINDOWS[value] && !splitRange(value)) return;
      if (btn.classList && (btn.classList.contains('segment-btn') || btn.classList.contains('tm-preset') || btn.classList.contains('tm-custom-range'))) {
        if (on) btn.classList.add('active');
        else btn.classList.remove('active');
      }
      if (value === 'custom') {
        btn.textContent = custom
          ? ('Custom (' + formatHumanWindow(splitRange(preset)[0], splitRange(preset)[1], preset) + ')')
          : 'Custom Range ▾';
      }
      if (btn.hasAttribute && (btn.hasAttribute('data-preset') || btn.hasAttribute('aria-pressed') || value === 'custom')) {
        btn.setAttribute('aria-pressed', on ? 'true' : 'false');
      }
    });
    var index = PRESET_ORDER.indexOf(preset);
    var segs = document.querySelectorAll('.tm-seg-control');
    Array.prototype.forEach.call(segs, function (seg) {
      if (!seg || !seg.classList) return;
      if (index < 0) seg.classList.add('tm-custom-active');
      else {
        seg.classList.remove('tm-custom-active');
        if (seg.style && typeof seg.style.setProperty === 'function') seg.style.setProperty('--tm-index', String(index));
      }
    });
  }

  function ensureActiveSlicePill() {
    if (typeof document === 'undefined' || typeof document.getElementById !== 'function') return null;
    var pill = document.getElementById('dendro-active-slice');
    if (pill) return pill;
    if (typeof document.createElement !== 'function' || typeof document.querySelector !== 'function') return null;
    var host = document.querySelector('.dendro-time-machine .tm-head')
      || document.querySelector('#hub-time-machine .time-machine-status')
      || document.querySelector('#hub-time-machine')
      || document.querySelector('.dendro-time-machine');
    if (!host || typeof host.appendChild !== 'function') return null;
    pill = document.createElement('p');
    pill.id = 'dendro-active-slice';
    pill.className = 'dendro-active-slice';
    host.appendChild(pill);
    return pill;
  }

  function paintReadouts(preset, payload) {
    if (typeof document === 'undefined' || typeof document.querySelectorAll !== 'function') return;
    var win = windowFrom(preset, payload);
    var human = win ? (win.display || formatHumanWindow(win.from_date || win.from, win.to_date || win.to, preset)) : '';
    var raw = win ? (String(win.from == null || win.from === '' ? '—' : win.from) + ' → ' + String(win.to == null || win.to === '' ? '—' : win.to)) : '';
    Array.prototype.forEach.call(document.querySelectorAll('.tm-window-readout'), function (node) {
      if (!node) return;
      node.textContent = human;
      if (raw && typeof node.setAttribute === 'function') node.setAttribute('title', raw);
    });
    Array.prototype.forEach.call(document.querySelectorAll('.tm-axis'), function (axis) {
      paintAxisTicks(axis, payload);
    });
    var hubBadge = typeof document.getElementById === 'function' ? document.getElementById('hub-window-badge') : null;
    if (hubBadge) hubBadge.textContent = activeSliceText(preset, payload).replace(/^⏱\s*/, '');
  }

  function paintAxisTicks(axis, payload) {
    if (!axis || typeof document === 'undefined' || typeof document.createElement !== 'function') return;
    while (axis.firstChild) axis.removeChild(axis.firstChild);
    var domain = domainMillis(payload);
    var steps = 4;
    var i;
    for (i = 0; i < steps; i++) {
      var at = domain.start + (domain.end - domain.start) * (i / (steps - 1));
      var iso = new Date(at).toISOString();
      var tick = document.createElement('span');
      tick.className = 'tm-tick';
      tick.textContent = formatHumanWindow(iso, iso);
      axis.appendChild(tick);
    }
  }

  function paintActiveSlice(preset, payload) {
    var current = preset || getPersistedWindow();
    var pill = ensureActiveSlicePill();
    if (pill) pill.textContent = activeSliceText(current, payload);
    try { paintReadouts(current, payload); } catch (err) {}
    try { highlightWindowButtons(current); } catch (err) {}
  }

  function writeWindowQuery(preset) {
    try {
      if (typeof history === 'undefined' || !history || typeof history.replaceState !== 'function') return;
      if (typeof location === 'undefined' || !location) return;
      var params = new URLSearchParams(typeof location.search === 'string' ? location.search : '');
      params.set('window', preset);
      var path = location.pathname || '';
      var hash = location.hash || '';
      var next = path + '?' + params.toString() + hash;
      var current = path + (location.search || '') + hash;
      if (current === next) return;
      history.replaceState(history.state, '', next);
    } catch (err) {}
  }

  function setPersistedWindow(preset) {
    if (!normalizeWindowPreset(preset)) return;
    try {
      if (typeof localStorage !== 'undefined' && localStorage && typeof localStorage.setItem === 'function') {
        localStorage.setItem('dendro_time_window', preset);
      }
    } catch (err) {}
    writeWindowQuery(preset);
    rewriteDashboardLinks(preset);
    highlightWindowButtons(preset);
    try {
      if (global.DendroState) {
        global.DendroState.windowPreset = preset;
        global.DendroState.preset = preset;
      }
    } catch (err) {}
    var payload = null;
    try { payload = DendroState.get().lastPayload; } catch (err) { payload = null; }
    paintActiveSlice(preset, payload);
  }

  function bindWindowChrome() {
    if (bindWindowChrome.bound) return;
    bindWindowChrome.bound = true;
    if (typeof document === 'undefined' || typeof document.addEventListener !== 'function') return;
    document.addEventListener('click', function (event) {
      var node = event && event.target;
      if (!node || typeof node.closest !== 'function') return;
      var customBtn = node.closest('[data-window="custom"]');
      if (!customBtn) return;
      if (event.preventDefault) event.preventDefault();
      if (event.stopPropagation) event.stopPropagation();
      openCustomRangePopover(customBtn, null);
    }, true);
    document.addEventListener('click', function (event) {
      if (!customPopover || customPopover.hidden) return;
      var node = event && event.target;
      if (!node) return;
      if (typeof customPopover.contains === 'function' && customPopover.contains(node)) return;
      if (typeof node.closest === 'function' && node.closest('[data-window="custom"]')) return;
      closeCustomRangePopover();
    });
    document.addEventListener('click', function (event) {
      var node = event && event.target;
      if (!node || typeof node.closest !== 'function') return;
      if (node.closest('.dendro-pivot-bar') || node.closest('.dendro-custom-range-popover') || node.closest('[data-window="custom"]')) return;
      var btn = node.closest('[data-window], [data-preset]');
      if (!btn || typeof btn.getAttribute !== 'function') return;
      var preset = btn.getAttribute('data-window') || btn.getAttribute('data-preset');
      if (!normalizePreset(preset)) return;
      try { setPersistedWindow(preset); } catch (err) {}
      if (dendroState.preset !== preset) {
        var bounds = rangeForPreset(preset);
        DendroState.set({
          preset: preset,
          range: { start: bounds[0], end: bounds[1] }
        });
      }
    });
  }

  function readStoredGroup() {
    try {
      if (typeof localStorage !== 'undefined' && localStorage && typeof localStorage.getItem === 'function') {
        var value = localStorage.getItem('dendro_group_by');
        if (value === 'harness' || value === 'capability' || value === 'date' || value === 'rig' || value === 'status' || value === 'raw') return value;
      }
    } catch (err) {}
    return 'harness';
  }

  function storeGroup(group) {
    try {
      if (typeof localStorage !== 'undefined' && localStorage && typeof localStorage.setItem === 'function') {
        localStorage.setItem('dendro_group_by', group);
      }
    } catch (err) {}
  }

  function canonicalPivotView(name) {
    var known = canonicalConsole(name);
    if (known) return known;
    if (typeof name !== 'string') return '';
    switch (name) {
      case 'trajectory_vortex':
      case 'v_trajectory_vortex':
        return 'trajectory_vortex';
      case 'schema_drift':
      case 'v_schema_drift':
        return 'schema_drift';
      case 'authority_blast_radius':
      case 'v_authority_blast_radius':
        return 'authority_blast_radius';
      case 'counterfactual_routing':
      case 'v_counterfactual_routing':
        return 'counterfactual_routing';
      case 'message_mix':
      case 'v_message_mix':
        return 'message_mix';
      case 'subagent_lineage':
      case 'v_subagent_lineage':
        return 'subagent_lineage';
      case 'security_posture':
      case 'v_security_posture':
        return 'security_posture';
      case 'process_resources':
      case 'v_process_resources':
        return 'process_resources';
      default:
        return '';
    }
  }

  function findConsoleTbody() {
    if (typeof document === 'undefined' || typeof document.getElementById !== 'function') return null;
    var i;
    for (i = 0; i < CONSOLE_TBODY_IDS.length; i++) {
      var node = document.getElementById(CONSOLE_TBODY_IDS[i]);
      if (node) return node;
    }
    if (typeof document.querySelector !== 'function') return null;
    var extra = document.querySelector('table.data-table tbody');
    if (extra && extra.id !== 'live-duckdb-body') return extra;
    return null;
  }

  function findRawCard() {
    var tbody = findConsoleTbody();
    if (!tbody) return null;
    if (typeof tbody.closest === 'function') {
      return tbody.closest('.table-panel') || tbody.closest('section') || (tbody.parentNode && tbody.parentNode.parentNode) || null;
    }
    return tbody.parentNode || null;
  }

  function firstNum(row, keys) {
    if (!row || typeof row !== 'object') return null;
    var i;
    for (i = 0; i < keys.length; i++) {
      if (row[keys[i]] == null || row[keys[i]] === '') continue;
      var n = Number(row[keys[i]]);
      if (isFinite(n)) return n;
    }
    return null;
  }

  function sumKeys(rows, keys) {
    var total = 0;
    var seen = false;
    rows.forEach(function (row) {
      var n = firstNum(row, keys);
      if (n == null) return;
      seen = true;
      total += n;
    });
    return seen ? total : null;
  }

  function avgKeys(rows, keys) {
    var total = 0;
    var count = 0;
    rows.forEach(function (row) {
      var n = firstNum(row, keys);
      if (n == null) return;
      total += n;
      count += 1;
    });
    return count ? total / count : null;
  }

  function maxKeys(rows, keys) {
    var max = null;
    rows.forEach(function (row) {
      var n = firstNum(row, keys);
      if (n == null) return;
      if (max == null || n > max) max = n;
    });
    return max;
  }

  function formatCompact(value) {
    if (value == null || !isFinite(value)) return '—';
    var abs = Math.abs(value);
    if (abs >= 1e9) return (value / 1e9).toFixed(2) + 'B';
    if (abs >= 1e6) return (value / 1e6).toFixed(2) + 'M';
    if (abs >= 1e3) return (value / 1e3).toFixed(1) + 'K';
    if (Math.round(value) !== value) return value.toFixed(2);
    return String(Math.round(value));
  }

  function formatBytes(value) {
    if (value == null || !isFinite(value)) return '—';
    if (Math.abs(value) >= 1048576) return (value / 1048576).toFixed(1) + ' MiB';
    if (Math.abs(value) >= 1024) return (value / 1024).toFixed(1) + ' KiB';
    return Math.round(value) + ' B';
  }

  function trippedFlag(row) {
    var value = row && row.tripped;
    if (value === true || value === 1) return true;
    return typeof value === 'string' && /^(true|yes|1|tripped)$/i.test(value.trim());
  }

  function groupKeyFor(row, group) {
    if (!row || typeof row !== 'object') return 'unknown';
    if (group === 'date') {
      var raw = row.date_bucket || row.ts || row.recorded_at || row.window_from || row.first_seen_at || '';
      var day = String(raw).slice(0, 10);
      return day || 'Active Window';
    }
    if (group === 'rig') return String(row.rig || row.workspace || row.attribution_bucket || row.boundary || row.lane || row.harness || 'default');
    if (group === 'status') {
      if (row.status) return String(row.status);
      if (row.kind) return String(row.kind);
      if (row.role) return String(row.role);
      return trippedFlag(row) ? 'VORTEX_TRIPPED' : 'NORMAL';
    }
    if (group === 'raw') return 'raw';
    if (group === 'capability') return capabilityFamily(row);
    return String(row.harness || row.model || row.seat_id || row.seat || row.from_seat || row.rig || 'unknown');
  }

  function capabilityFamily(row) {
    if (!row || typeof row !== 'object') return 'Other Tools';
    var explicit = row.tool_family || row.capability_family || row.family || row.archetype;
    if (explicit) return String(explicit);
    var name = String(row.tool || row.tool_name || row.capability || row.capability_id || row.skill || row.skill_id || row.action || '');
    var text = name.toLowerCase();
    if (!text) return 'Unspecified';
    if (/grep|glob|shell|bash|read_file|write|edit|fs\b|filesystem/.test(text)) return 'Shell & Files';
    if (/web_search|web|http|fetch|browse/.test(text)) return 'Web & Research';
    if (/git|commit|diff|branch|pull request/.test(text)) return 'Source Control';
    if (/sql|duck|query|table|schema/.test(text)) return 'Data & Query';
    if (/agent|handoff|queue|seat|subagent|delegate/.test(text)) return 'Coordination';
    if (/mcp|tool/.test(text)) return 'Tool Protocol';
    return 'Other Tools';
  }

  function recordsFromPayload(payload) {
    var keys = ['rows', 'missions', 'seats', 'pairs', 'records'];
    var i;
    if (payload && typeof payload === 'object' && !Array.isArray(payload)) {
      var empty = null;
      for (i = 0; i < keys.length; i++) {
        if (!Array.isArray(payload[keys[i]])) continue;
        if (payload[keys[i]].length) return payload[keys[i]];
        if (!empty) empty = payload[keys[i]];
      }
      if (empty) return empty;
    }
    try {
      if (typeof document === 'undefined' || typeof document.getElementById !== 'function') return [];
      var el = document.getElementById('dendro-data');
      if (!el || !el.textContent) return [];
      var text = el.textContent.trim();
      if (!text || text === 'null') return [];
      var parsed = JSON.parse(text);
      if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) return [];
      for (i = 0; i < keys.length; i++) {
        if (Array.isArray(parsed[keys[i]]) && parsed[keys[i]].length) return parsed[keys[i]];
      }
    } catch (err) {}
    return [];
  }

  function metricPair(view, rows) {
    if (view === 'cost_and_loops') {
      var tokens = sumKeys(rows, ['total_tokens']);
      var cache = sumKeys(rows, ['cache_read', 'cache_read_tokens']);
      var fresh = sumKeys(rows, ['input_uncached', 'fresh_input_tokens', 'input_tokens']);
      var write = sumKeys(rows, ['cache_write', 'cache_write_tokens']);
      var den = (fresh || 0) + (cache || 0) + (write || 0);
      return [
        { label: 'Total Tokens', value: formatCompact(tokens) },
        { label: 'Cache Hit %', value: den ? (((cache || 0) / den) * 100).toFixed(1) + '%' : '—' }
      ];
    }
    if (view === 'seat_dynamics') {
      var observed = sumKeys(rows, ['observed_secs']);
      var active = sumKeys(rows, ['active_secs']);
      return [
        { label: 'Active %', value: observed ? (((active || 0) / observed) * 100).toFixed(1) + '%' : '—' },
        { label: 'M1 Yield', value: formatCompact(avgKeys(rows, ['m1_yield'])) }
      ];
    }
    if (view === 'handoff_flow') {
      var handoffs = sumKeys(rows, ['handoffs', 'count']);
      var rework = sumKeys(rows, ['rework_returns']);
      return [
        { label: 'Total Handoffs', value: formatCompact(handoffs) },
        { label: 'Return Rate %', value: handoffs ? (((rework || 0) / handoffs) * 100).toFixed(1) + '%' : '—' }
      ];
    }
    if (view === 'trajectory_vortex') {
      var tripped = 0;
      rows.forEach(function (row) { if (trippedFlag(row)) tripped += 1; });
      var turn = avgKeys(rows, ['turn_k']);
      return [
        { label: 'Vortex Tripped', value: String(tripped) },
        { label: 'Avg Turn K', value: turn == null ? '—' : turn.toFixed(2) }
      ];
    }
    if (view === 'schema_drift') {
      var l1 = avgKeys(rows, ['l1_distance', 'l1_drift', 'drift']);
      var keysChanged = 0;
      rows.forEach(function (row) {
        var explicit = firstNum(row, ['keys_changed']);
        if (explicit != null) keysChanged += explicit;
        else keysChanged += (firstNum(row, ['keys_added']) || 0) + (firstNum(row, ['keys_removed']) || 0) + (firstNum(row, ['keys_retyped']) || 0);
      });
      return [
        { label: 'Avg L1 Drift', value: l1 == null ? '—' : l1.toFixed(3) },
        { label: 'Keys Changed', value: String(Math.round(keysChanged)) }
      ];
    }
    if (view === 'authority_blast_radius') {
      var hop = maxKeys(rows, ['max_hop', 'hop']);
      return [
        { label: 'Downstream Blast Fanout', value: formatCompact(sumKeys(rows, ['downstream_sessions', 'fanout', 'downstream'])) },
        { label: 'Max Hop', value: hop == null ? '—' : String(hop) }
      ];
    }
    if (view === 'counterfactual_routing') {
      var pressure = avgKeys(rows, ['observed_context_pressure', 'context_pressure', 'simulated_context_pressure', 'cache_bleed_inflection']);
      return [
        { label: 'Observed Tokens', value: formatCompact(sumKeys(rows, ['observed_tokens', 'total_tokens', 'tokens'])) },
        { label: 'Avg Context Pressure', value: pressure == null ? '—' : pressure.toFixed(3) }
      ];
    }
    if (view === 'message_mix') {
      return [
        { label: 'Message Count', value: formatCompact(sumKeys(rows, ['message_count', 'count'])) },
        { label: 'Rows', value: String(rows.length) }
      ];
    }
    if (view === 'subagent_lineage') {
      return [
        { label: 'Child Sessions', value: formatCompact(sumKeys(rows, ['child_count', 'children'])) },
        { label: 'Failed Count', value: formatCompact(sumKeys(rows, ['failed_count', 'failed'])) }
      ];
    }
    if (view === 'security_posture') {
      var observations = sumKeys(rows, ['observation_count']);
      return [
        { label: 'Security Observations', value: observations == null ? String(rows.length) : formatCompact(observations) },
        { label: 'Sessions', value: formatCompact(sumKeys(rows, ['session_count'])) }
      ];
    }
    if (view === 'process_resources') {
      return [
        { label: 'Max RSS', value: formatBytes(maxKeys(rows, ['max_rss_bytes', 'max_rss', 'rss'])) },
        { label: 'Process Count', value: formatCompact(sumKeys(rows, ['process_count'])) }
      ];
    }
    if (view === 'executive_governance') {
      return [
        { label: 'Records', value: String(rows.length) },
        { label: 'M1 Yield', value: formatCompact(avgKeys(rows, ['m1_yield'])) }
      ];
    }
    return [
      { label: 'Records', value: String(rows.length) },
      { label: 'Groups', value: '1' }
    ];
  }

  function dispatchInput(input) {
    if (!input) return;
    try {
      if (typeof Event === 'function') {
        input.dispatchEvent(new Event('input', { bubbles: true }));
        return;
      }
    } catch (err) {}
    try {
      if (typeof document !== 'undefined' && typeof document.createEvent === 'function') {
        var evt = document.createEvent('Event');
        evt.initEvent('input', true, true);
        input.dispatchEvent(evt);
      }
    } catch (err2) {}
  }

  function collectSearchInputs() {
    if (typeof document === 'undefined' || typeof document.querySelectorAll !== 'function') return [];
    return document.querySelectorAll('input.search-input, input[type="search"], #burn-search, #seat-search, #handoff-search, #table-search, #view-search');
  }

  function applyPivotFilter() {
    var key = pivotState.filterKey;
    var records = pivotState.records || [];
    var tbody = findConsoleTbody();
    var inputs = collectSearchInputs();
    function hideByIndex() {
      if (!tbody || typeof tbody.querySelectorAll !== 'function') return;
      var trs = tbody.querySelectorAll('tr');
      if (trs.length && trs.length === records.length) {
        Array.prototype.forEach.call(trs, function (tr, index) {
          var match = !key || groupKeyFor(records[index], pivotState.group) === key;
          tr.hidden = !match;
        });
        return;
      }
      Array.prototype.forEach.call(trs, function (tr) {
        if (!key) {
          tr.hidden = false;
          return;
        }
        var text = (tr.textContent || '').toLowerCase();
        tr.hidden = text.indexOf(String(key).toLowerCase()) === -1;
      });
    }
    if (!key) {
      Array.prototype.forEach.call(inputs, function (input) {
        if (!input || input.getAttribute('data-dendro-pivot-search') !== '1') return;
        input.value = '';
        input.removeAttribute('data-dendro-pivot-search');
        dispatchInput(input);
      });
      hideByIndex();
      return;
    }
    var before = tbody && tbody.querySelectorAll ? tbody.querySelectorAll('tr').length : 0;
    Array.prototype.forEach.call(inputs, function (input) {
      if (!input) return;
      input.setAttribute('data-dendro-pivot-search', '1');
      input.value = key;
      dispatchInput(input);
    });
    var afterRows = tbody && tbody.querySelectorAll ? tbody.querySelectorAll('tr') : [];
    var visible = 0;
    Array.prototype.forEach.call(afterRows, function (tr) {
      if (!tr.hidden) visible += 1;
    });
    if (tbody && before === records.length && afterRows.length === records.length) {
      hideByIndex();
      return;
    }
    if (key && visible === 0) {
      Array.prototype.forEach.call(inputs, function (input) {
        if (!input) return;
        input.value = '';
        dispatchInput(input);
      });
      hideByIndex();
    }
  }

  function syncRawCard() {
    var card = findRawCard();
    if (card && card.classList) {
      if (pivotState.rawOpen) card.classList.remove('dendro-raw-collapsed');
      else card.classList.add('dendro-raw-collapsed');
    }
    var toggle = typeof document !== 'undefined' && document.getElementById ? document.getElementById('dendro-raw-toggle') : null;
    if (toggle) {
      toggle.setAttribute('aria-expanded', pivotState.rawOpen ? 'true' : 'false');
      toggle.textContent = pivotState.rawOpen ? 'Hide Raw Records' : 'Show/Filter Raw Records';
    }
  }

  function syncPivotSeg() {
    if (typeof document === 'undefined' || typeof document.getElementById !== 'function') return;
    var bar = document.getElementById('dendro-pivot-bar');
    if (!bar || typeof bar.querySelectorAll !== 'function') return;
    var index = 0;
    PIVOT_GROUPS.forEach(function (group, i) {
      if (group.id === pivotState.group) index = i;
    });
    var seg = bar.querySelector('.pivot-seg');
    if (seg && seg.style) seg.style.setProperty('--pivot-index', String(index));
    Array.prototype.forEach.call(bar.querySelectorAll('[data-group]'), function (btn) {
      var on = btn.getAttribute('data-group') === pivotState.group;
      btn.setAttribute('aria-pressed', on ? 'true' : 'false');
    });
    syncRawCard();
  }

  function appendCell(row, text, className) {
    var cell = document.createElement('td');
    if (className) cell.className = className;
    cell.textContent = text == null || text === '' ? '—' : String(text);
    row.appendChild(cell);
  }

  function renderDrillTable(records) {
    var wrap = document.createElement('div');
    wrap.className = 'pivot-drill-table';
    var table = document.createElement('table');
    var sample = records[0] || {};
    var columns = [];
    Object.keys(sample).forEach(function (key) {
      if (columns.length >= 6) return;
      var value = sample[key];
      if (value != null && typeof value === 'object') return;
      columns.push(key);
    });
    if (!columns.length) columns = ['value'];
    var head = document.createElement('thead');
    var headRow = document.createElement('tr');
    columns.forEach(function (key) {
      var th = document.createElement('th');
      th.textContent = key;
      headRow.appendChild(th);
    });
    head.appendChild(headRow);
    var body = document.createElement('tbody');
    records.slice(0, 8).forEach(function (record) {
      var tr = document.createElement('tr');
      columns.forEach(function (key) {
        var value = record ? record[key] : '';
        if (value != null && typeof value === 'object') {
          try { value = JSON.stringify(value); } catch (err) { value = ''; }
        }
        appendCell(tr, value);
      });
      body.appendChild(tr);
    });
    table.appendChild(head);
    table.appendChild(body);
    wrap.appendChild(table);
    if (records.length > 8) {
      var more = document.createElement('p');
      more.className = 'pivot-more';
      more.textContent = (records.length - 8) + ' more records in this group. Filter the raw table to see them all.';
      wrap.appendChild(more);
    }
    return wrap;
  }

  function renderCapabilityStudio(order, buckets, total) {
    var studio = document.createElement('section');
    studio.className = 'capability-studio';
    studio.setAttribute('aria-label', 'Capability and tool families');
    var tabs = document.createElement('div');
    tabs.className = 'cap-tabs';
    tabs.setAttribute('role', 'tablist');
    var bar = document.createElement('div');
    bar.className = 'archetype-bar';
    var grid = document.createElement('div');
    grid.className = 'cap-bento-grid';
    order.forEach(function (key) {
      var members = buckets[key];
      var share = (members.length / total) * 100;
      var tab = document.createElement('button');
      tab.type = 'button';
      tab.className = 'cap-pill';
      tab.setAttribute('data-group-key', key);
      tab.setAttribute('aria-pressed', pivotState.filterKey === key ? 'true' : 'false');
      tab.textContent = key;
      tabs.appendChild(tab);
      var segment = document.createElement('span');
      segment.className = 'cap-pill';
      segment.style.width = Math.max(8, Math.min(100, share)).toFixed(1) + '%';
      segment.textContent = key;
      bar.appendChild(segment);
      var card = document.createElement('article');
      card.className = 'cap-bento-card';
      var heading = document.createElement('h3');
      heading.textContent = key;
      var count = document.createElement('p');
      count.textContent = members.length + (members.length === 1 ? ' record' : ' records') + ' · ' + share.toFixed(1) + '%';
      card.appendChild(heading);
      card.appendChild(count);
      grid.appendChild(card);
    });
    studio.appendChild(tabs);
    studio.appendChild(bar);
    studio.appendChild(grid);
    return studio;
  }

  function renderGroupedRollup(view, records) {
    var rollup = document.getElementById('dendro-grouped-rollup');
    if (!rollup) return;
    while (rollup.firstChild) rollup.removeChild(rollup.firstChild);
    if (pivotState.group === 'raw') {
      var rawNote = document.createElement('p');
      rawNote.className = 'pivot-raw-note';
      rawNote.textContent = records.length + ' raw records in this slice. Choose a dimension above to roll them up.';
      rollup.appendChild(rawNote);
      return;
    }
    if (!records.length) {
      var empty = document.createElement('p');
      empty.className = 'pivot-raw-note';
      empty.textContent = 'No records in this slice to roll up.';
      rollup.appendChild(empty);
      return;
    }
    var buckets = {};
    var order = [];
    records.forEach(function (record) {
      var key = groupKeyFor(record, pivotState.group);
      if (!buckets[key]) {
        buckets[key] = [];
        order.push(key);
      }
      buckets[key].push(record);
    });
    order.sort(function (a, b) { return buckets[b].length - buckets[a].length || (a < b ? -1 : a > b ? 1 : 0); });
    var total = records.length || 1;
    if (pivotState.group === 'capability') rollup.appendChild(renderCapabilityStudio(order, buckets, total));
    var matrix = document.createElement('div');
    matrix.className = 'pivot-matrix';
    matrix.setAttribute('role', 'table');
    matrix.setAttribute('aria-label', 'Grouped rollup');
    var head = document.createElement('div');
    head.className = 'pivot-matrix-head';
    head.setAttribute('role', 'row');
    ['Group', 'Records', 'Share of fleet', 'Primary metric', 'Secondary metric'].forEach(function (label) {
      var span = document.createElement('span');
      span.setAttribute('role', 'columnheader');
      span.textContent = label;
      head.appendChild(span);
    });
    matrix.appendChild(head);
    order.forEach(function (key) {
      var members = buckets[key];
      var share = (members.length / total) * 100;
      var metrics = metricPair(view, members);
      var rowBtn = document.createElement('button');
      rowBtn.type = 'button';
      rowBtn.className = 'pivot-group-row';
      rowBtn.setAttribute('role', 'row');
      rowBtn.setAttribute('data-group-key', key);
      rowBtn.setAttribute('aria-pressed', pivotState.filterKey === key ? 'true' : 'false');
      var name = document.createElement('span');
      name.className = pivotState.group === 'capability' ? 'pivot-group-name cap-pill' : 'pivot-group-name';
      name.textContent = key;
      var count = document.createElement('span');
      count.className = 'pivot-group-count';
      count.textContent = String(members.length);
      var shareCell = document.createElement('span');
      shareCell.className = 'pivot-share-cell';
      var bar = document.createElement('span');
      bar.className = 'pivot-share';
      var fill = document.createElement('span');
      fill.style.width = Math.max(2, Math.min(100, share)).toFixed(1) + '%';
      bar.appendChild(fill);
      var pct = document.createElement('span');
      pct.className = 'pivot-share-pct';
      pct.textContent = share.toFixed(1) + '%';
      shareCell.appendChild(bar);
      shareCell.appendChild(pct);
      var primary = document.createElement('span');
      primary.className = 'pivot-metric';
      primary.textContent = metrics[0].label + ' ' + metrics[0].value;
      var secondary = document.createElement('span');
      secondary.className = 'pivot-metric';
      secondary.textContent = metrics[1].label + ' ' + metrics[1].value;
      rowBtn.appendChild(name);
      rowBtn.appendChild(count);
      rowBtn.appendChild(shareCell);
      rowBtn.appendChild(primary);
      rowBtn.appendChild(secondary);
      matrix.appendChild(rowBtn);
      var drill = document.createElement('div');
      drill.className = 'pivot-drill';
      drill.hidden = pivotState.expandedKey !== key;
      var drillBtn = document.createElement('button');
      drillBtn.type = 'button';
      drillBtn.className = 'pivot-drill-toggle';
      drillBtn.setAttribute('data-drill-key', key);
      drillBtn.setAttribute('aria-expanded', pivotState.expandedKey === key ? 'true' : 'false');
      drillBtn.textContent = pivotState.expandedKey === key ? 'Hide grouped records' : 'Drill into ' + members.length + ' records';
      drill.appendChild(drillBtn);
      if (pivotState.expandedKey === key) drill.appendChild(renderDrillTable(members));
      matrix.appendChild(drill);
    });
    rollup.appendChild(matrix);
  }

  function refreshDimensionPivot(payload, viewId) {
    try {
      if (typeof document === 'undefined' || typeof document.getElementById !== 'function') return;
      if (!document.getElementById('dendro-pivot-bar')) return;
      if (viewId) pivotState.viewId = canonicalPivotView(viewId) || viewId;
      var view = canonicalPivotView(pivotState.viewId) || canonicalPivotView(payload && payload.view) || pivotState.viewId || '';
      pivotState.records = recordsFromPayload(payload);
      if (pivotState.filterKey && pivotState.group !== 'raw') {
        var still = false;
        var i;
        for (i = 0; i < pivotState.records.length; i++) {
          if (groupKeyFor(pivotState.records[i], pivotState.group) === pivotState.filterKey) {
            still = true;
            break;
          }
        }
        if (!still) pivotState.filterKey = null;
      }
      renderGroupedRollup(view, pivotState.records);
      syncPivotSeg();
      if (pivotState.group !== 'raw') applyPivotFilter();
    } catch (err) {}
  }

  function selectPivotGroup(group) {
    if (group !== 'harness' && group !== 'capability' && group !== 'date' && group !== 'rig' && group !== 'status' && group !== 'raw') return;
    pivotState.group = group;
    pivotState.filterKey = null;
    pivotState.expandedKey = null;
    storeGroup(group);
    if (group === 'raw') pivotState.rawOpen = true;
    syncPivotSeg();
    var payload = null;
    try { payload = DendroState.get().lastPayload; } catch (err) { payload = null; }
    refreshDimensionPivot(payload, pivotState.viewId);
    applyPivotFilter();
  }

  function mountDimensionPivotBar(viewId) {
    var doc = typeof document !== 'undefined' ? document : null;
    if (doc && typeof doc.querySelector === 'function' && doc.querySelector('.rollup-group-bar')) return null;
    try {
      if (typeof document === 'undefined' || typeof document.createElement !== 'function' || typeof document.getElementById !== 'function') return null;
      var tbody = findConsoleTbody();
      if (!tbody) return null;
      var table = typeof tbody.closest === 'function' ? tbody.closest('table') : tbody.parentNode;
      var card = findRawCard();
      if (!table || !card || !card.parentNode || typeof card.parentNode.insertBefore !== 'function') return null;
      pivotState.viewId = canonicalPivotView(viewId) || viewId || pivotState.viewId;
      pivotState.group = readStoredGroup();
      var bar = document.getElementById('dendro-pivot-bar');
      if (!bar) {
        bar = document.createElement('section');
        bar.id = 'dendro-pivot-bar';
        bar.className = 'dendro-pivot-bar glass';
        bar.setAttribute('aria-label', 'Dimension pivot and rollup');
        var head = document.createElement('div');
        head.className = 'pivot-head';
        var title = document.createElement('div');
        title.className = 'pivot-title';
        title.textContent = 'Dimension Grouping & Rollup';
        var toggle = document.createElement('button');
        toggle.type = 'button';
        toggle.id = 'dendro-raw-toggle';
        toggle.className = 'pivot-raw-toggle';
        toggle.textContent = 'Show/Filter Raw Records';
        toggle.addEventListener('click', function () {
          pivotState.rawOpen = !pivotState.rawOpen;
          syncRawCard();
        });
        head.appendChild(title);
        head.appendChild(toggle);
        var seg = document.createElement('div');
        seg.className = 'pivot-seg';
        seg.setAttribute('role', 'tablist');
        seg.setAttribute('aria-label', 'Group by dimension');
        var pill = document.createElement('span');
        pill.className = 'pivot-seg-pill';
        pill.setAttribute('aria-hidden', 'true');
        seg.appendChild(pill);
        PIVOT_GROUPS.forEach(function (group) {
          var button = document.createElement('button');
          button.type = 'button';
          button.setAttribute('data-group', group.id);
          button.setAttribute('aria-pressed', 'false');
          button.textContent = group.label;
          button.addEventListener('click', function () { selectPivotGroup(group.id); });
          seg.appendChild(button);
        });
        var rollup = document.createElement('div');
        rollup.id = 'dendro-grouped-rollup';
        rollup.className = 'dendro-grouped-rollup';
        rollup.addEventListener('click', function (event) {
          var node = event && event.target;
          if (!node || typeof node.closest !== 'function') return;
          var drill = node.closest('[data-drill-key]');
          if (drill) {
            var drillKey = drill.getAttribute('data-drill-key');
            pivotState.expandedKey = pivotState.expandedKey === drillKey ? null : drillKey;
            refreshDimensionPivot(DendroState.get().lastPayload, pivotState.viewId);
            return;
          }
          var row = node.closest('[data-group-key]');
          if (!row || pivotState.group === 'raw') return;
          var key = row.getAttribute('data-group-key');
          pivotState.filterKey = pivotState.filterKey === key ? null : key;
          if (pivotState.filterKey) pivotState.rawOpen = true;
          syncRawCard();
          applyPivotFilter();
          renderGroupedRollup(canonicalPivotView(pivotState.viewId), pivotState.records);
        });
        bar.appendChild(head);
        bar.appendChild(seg);
        bar.appendChild(rollup);
        card.parentNode.insertBefore(bar, card);
      }
      refreshDimensionPivot(null, pivotState.viewId);
      return bar;
    } catch (err) {
      return null;
    }
  }

  function initSprings() {
    try {
      if (typeof document === 'undefined' || !document.documentElement || !document.documentElement.style || typeof document.documentElement.style.setProperty !== 'function') return;
      var root = document.documentElement;
      root.style.setProperty('--spring-snappy', simulateSpring(260, 28, 1));
      root.style.setProperty('--spring-gentle', simulateSpring(78, 16, 1));
      root.style.setProperty('--spring-bouncy', simulateSpring(170, 7, 1));
      if (typeof document.addEventListener !== 'function') return;
      document.addEventListener('pointermove', function (event) {
        if (!event || typeof document.querySelectorAll !== 'function') return;
        var nodes = document.querySelectorAll('.glass');
        for (var i = 0; i < nodes.length; i++) {
          if (!nodes[i].getBoundingClientRect || !nodes[i].style) continue;
          var rect = nodes[i].getBoundingClientRect();
          nodes[i].style.setProperty('--mx', (event.clientX - rect.left) + 'px');
          nodes[i].style.setProperty('--my', (event.clientY - rect.top) + 'px');
        }
      }, { passive: true });
    } catch (err) {}
  }

  function nestLivePanelInDrawer() {
    if (typeof document === 'undefined' || typeof document.getElementById !== 'function' || typeof document.createElement !== 'function') return;
    var panel = document.getElementById('live-duckdb-panel');
    if (!panel || !panel.parentNode) return;
    var details = typeof panel.closest === 'function' ? panel.closest('.dendro-l3-drawer') : null;
    if (!details && panel.parentNode.classList && panel.parentNode.classList.contains('dendro-l3-drawer')) details = panel.parentNode;
    if (!details) {
      details = document.createElement('details');
      details.className = 'dendro-l3-drawer';
      var summary = document.createElement('summary');
      var label = document.createElement('span');
      label.textContent = 'Data Lineage, Stack Coverage & SQL Proof';
      var pill = document.createElement('span');
      pill.className = 'l3-pill';
      pill.textContent = 'L3 Proof · </>';
      summary.appendChild(label);
      summary.appendChild(pill);
      details.appendChild(summary);
      panel.parentNode.insertBefore(details, panel);
      details.appendChild(panel);
    }
    if (typeof document.querySelectorAll === 'function') {
      Array.prototype.forEach.call(document.querySelectorAll('.query-box'), function (box) {
        if (box && box.parentNode !== details) details.appendChild(box);
      });
    }
    var host = null;
    if (typeof document.querySelector === 'function') {
      host = document.querySelector('.main-wrapper') || document.querySelector('main.telemetry-layout') || document.querySelector('main');
    }
    if (host && typeof host.appendChild === 'function' && details.parentNode !== host) host.appendChild(details);
  }

  var scrubberOpts = null;
  var scrubberFetchGen = 0;
  var PRESET_WINDOWS = {
    today: [0.9, 1],
    '24h': [0.9, 1],
    '1h': [0.98, 1],
    '7d': [0.72, 1],
    '30d': [0.5, 1],
    this_month: [0.66, 1],
    last_month: [0.33, 0.66],
    all: [0, 1]
  };
  var customPopover = null;

  function utcTodayYmd() {
    return ymdFromMs(Date.now());
  }

  function shiftYmd(ymd, days) {
    var clock = parseClock(ymd);
    if (!clock) return utcTodayYmd();
    return ymdFromMs(clock.ms + days * 86400000);
  }

  function fetchWindowQuery(preset) {
    if (typeof fetch !== 'function' || !preset) return;
    var view = (scrubberOpts && scrubberOpts.viewId) || 'hub';
    var base = (scrubberOpts && scrubberOpts.pollEndpoint) || '/api/query';
    var url = base + (base.indexOf('?') >= 0 ? '&' : '?') + 'view=' + encodeURIComponent(view) + '&window=' + encodeURIComponent(preset);
    fetch(url, { cache: 'no-store' }).then(function (res) {
      if (!res || !res.ok || typeof res.json !== 'function') return null;
      return res.json();
    }).then(function (data) {
      if (data == null) return;
      if (scrubberOpts && typeof scrubberOpts.publishLive === 'function') scrubberOpts.publishLive(data);
    }).catch(function () {});
  }

  function closeCustomRangePopover() {
    if (customPopover) customPopover.hidden = true;
  }

  function applyCustomSlice(fromDate, toDate) {
    var fromClock = parseClock(fromDate);
    var toClock = parseClock(toDate);
    if (!fromClock || !toClock) return;
    var from = ymdFromMs(fromClock.ms);
    var to = ymdFromMs(toClock.ms);
    if (from > to) {
      var swap = from;
      from = to;
      to = swap;
    }
    var preset = from + '..' + to;
    var bounds = rangeForPreset(preset);
    try {
      DendroState.set({ preset: preset, range: { start: bounds[0], end: bounds[1] } });
    } catch (err) {
      try { setPersistedWindow(preset); } catch (err2) {}
    }
    try {
      if (scrubberOpts && typeof scrubberOpts.reconnectForWindow === 'function') scrubberOpts.reconnectForWindow(preset);
    } catch (err3) {}
    fetchWindowQuery(preset);
    closeCustomRangePopover();
  }

  function commitCustomRange(fromDate, toDate) {
    if (customPopover && typeof customPopover._onApply === 'function') {
      customPopover._onApply(fromDate, toDate);
      closeCustomRangePopover();
      return;
    }
    applyCustomSlice(fromDate, toDate);
  }

  function presetDateBounds() {
    var payload = latestPayload();
    var preset = '';
    try { preset = (DendroState.get().preset) || getPersistedWindow(); }
    catch (err) { preset = 'all'; }
    var win = windowFrom(preset, payload) || {};
    var today = utcTodayYmd();
    var from = parseClock(win.from_date) || parseClock(win.from);
    var to = parseClock(win.to_date) || parseClock(win.to);
    if (!win.to_date && isExactUtcMidnight(to) && from && to.ms > from.ms) {
      to = parseClock(new Date(to.ms - 86400000).toISOString()) || to;
    }
    return {
      from: from ? ymdFromMs(from.ms) : today,
      to: to ? ymdFromMs(to.ms) : today
    };
  }

  function ensureCustomPopover() {
    if (customPopover) return customPopover;
    if (typeof document === 'undefined' || typeof document.createElement !== 'function' || !document.body) return null;
    var pop = document.createElement('div');
    pop.className = 'dendro-custom-range-popover';
    pop.hidden = true;
    pop.setAttribute('role', 'dialog');
    pop.setAttribute('aria-label', 'Custom date range');
    function dateField(label, attr) {
      var field = document.createElement('label');
      field.className = 'custom-date-field';
      field.appendChild(document.createTextNode(label));
      var input = document.createElement('input');
      input.type = 'date';
      input.setAttribute(attr, '1');
      field.appendChild(input);
      return field;
    }
    var fromField = dateField('From Date', 'data-custom-from');
    var toField = dateField('To Date', 'data-custom-to');
    var helpers = document.createElement('div');
    helpers.className = 'custom-range-helpers';
    function addHelper(label, fill) {
      var button = document.createElement('button');
      button.type = 'button';
      button.textContent = label;
      button.addEventListener('click', function () {
        var dates = fill();
        var fromInput = pop.querySelector('[data-custom-from]');
        var toInput = pop.querySelector('[data-custom-to]');
        if (fromInput) fromInput.value = dates[0];
        if (toInput) toInput.value = dates[1];
        commitCustomRange(dates[0], dates[1]);
      });
      helpers.appendChild(button);
    }
    addHelper('Last 3 Days', function () {
      var today = utcTodayYmd();
      return [shiftYmd(today, -2), today];
    });
    addHelper('Last 14 Days', function () {
      var today = utcTodayYmd();
      return [shiftYmd(today, -13), today];
    });
    addHelper('Oct 2026', function () {
      return ['2026-10-01', '2026-10-31'];
    });
    var apply = document.createElement('button');
    apply.type = 'button';
    apply.className = 'custom-apply-btn';
    apply.textContent = 'Apply Custom Slice';
    apply.addEventListener('click', function () {
      var fromInput = pop.querySelector('[data-custom-from]');
      var toInput = pop.querySelector('[data-custom-to]');
      commitCustomRange(fromInput && fromInput.value, toInput && toInput.value);
    });
    pop.appendChild(fromField);
    pop.appendChild(toField);
    pop.appendChild(helpers);
    pop.appendChild(apply);
    document.body.appendChild(pop);
    customPopover = pop;
    return pop;
  }

  function openCustomRangePopover(anchorEl, onApply) {
    var pop = ensureCustomPopover();
    if (!pop) return null;
    var bounds = presetDateBounds();
    var fromInput = pop.querySelector('[data-custom-from]');
    var toInput = pop.querySelector('[data-custom-to]');
    if (fromInput) fromInput.value = bounds.from;
    if (toInput) toInput.value = bounds.to;
    pop._onApply = typeof onApply === 'function' ? onApply : null;
    pop.hidden = false;
    if (pop.style) {
      pop.style.position = 'fixed';
      pop.style.zIndex = '80';
      if (anchorEl && typeof anchorEl.getBoundingClientRect === 'function') {
        var rect = anchorEl.getBoundingClientRect();
        var width = (typeof window !== 'undefined' && window.innerWidth) || 1024;
        pop.style.top = Math.round(rect.bottom + 8) + 'px';
        pop.style.left = Math.max(8, Math.round(Math.min(rect.left, width - 320))) + 'px';
      }
    }
    if (fromInput && typeof fromInput.focus === 'function') fromInput.focus();
    return pop;
  }

  function showBrushedApply(root, start, end) {
    if (!root || typeof root.querySelector !== 'function') return;
    var button = root.querySelector('.tm-brush-apply');
    if (!button) return;
    if (!(end > start + 0.005) || (start <= 0.01 && end >= 0.99)) {
      button.hidden = true;
      return;
    }
    var dates = datesFromBrush(start, end, latestPayload());
    var label = formatHumanWindow(dates[0], dates[1], dates[0] + '..' + dates[1]);
    button.hidden = false;
    button.textContent = 'Apply Brushed Dates (' + label + ')';
    button.onclick = function () {
      applyCustomSlice(dates[0], dates[1]);
      button.hidden = true;
    };
  }

  function augmentTimeMachine(root) {
    if (!root || typeof document === 'undefined' || typeof document.createElement !== 'function') return;
    if (typeof root.querySelector !== 'function') return;
    if (!root.querySelector('[data-window="custom"]')) {
      var button = document.createElement('button');
      button.type = 'button';
      button.className = 'tm-custom-range';
      button.setAttribute('data-window', 'custom');
      button.setAttribute('data-preset', 'custom');
      button.setAttribute('aria-haspopup', 'dialog');
      button.textContent = 'Custom Range ▾';
      var seg = root.querySelector('.segmented-control') || root.querySelector('.tm-seg-control');
      if (seg && seg.parentNode) seg.parentNode.insertBefore(button, seg.nextSibling);
      else root.appendChild(button);
    }
    if (!root.querySelector('.tm-window-readout')) {
      var readout = document.createElement('p');
      readout.className = 'tm-window-readout';
      var status = root.querySelector('.time-machine-status') || root.querySelector('.tm-head') || root;
      status.appendChild(readout);
    }
    if (!root.querySelector('.tm-axis')) {
      var axis = document.createElement('div');
      axis.className = 'tm-axis';
      axis.setAttribute('aria-hidden', 'true');
      root.appendChild(axis);
    }
    if (!root.querySelector('.tm-brush-apply')) {
      var apply = document.createElement('button');
      apply.type = 'button';
      apply.className = 'tm-brush-apply';
      apply.hidden = true;
      root.appendChild(apply);
    }
    if (root.getAttribute && root.getAttribute('data-brush-apply') !== '1' && typeof document.getElementById === 'function') {
      var left = document.getElementById('handle-left');
      var right = document.getElementById('handle-right');
      if (left && right && left.addEventListener && root.contains && root.contains(left)) {
        root.setAttribute('data-brush-apply', '1');
        var onUp = function () {
          var sel = document.getElementById('time-machine-selection');
          if (!sel || !sel.style) return;
          var brushStart = parseFloat(sel.style.left) / 100;
          var width = parseFloat(sel.style.width) / 100;
          if (!isFinite(brushStart) || !isFinite(width)) return;
          showBrushedApply(root, brushStart, brushStart + width);
        };
        left.addEventListener('pointerup', onUp);
        right.addEventListener('pointerup', onUp);
      }
    }
  }

  function decorateHubBento() {
    if (typeof document === 'undefined' || typeof document.querySelector !== 'function') return;
    var layers = document.querySelector('.architecture-layers');
    if (layers && layers.classList && !layers.classList.contains('layer-bento-dock')) layers.classList.add('layer-bento-dock');
    var persona = typeof document.getElementById === 'function' ? document.getElementById('persona-questions') : null;
    if (persona && persona.classList && !persona.classList.contains('persona-deck')) persona.classList.add('persona-deck');
    var insights = typeof document.getElementById === 'function' ? document.getElementById('compiled-insights') : null;
    if (insights && insights.classList && !insights.classList.contains('insights-matrix-deck')) insights.classList.add('insights-matrix-deck');
  }

  function bootDashboardChrome() {
    try {
      if (typeof document === 'undefined' || typeof document.querySelectorAll !== 'function') return;
      decorateHubBento();
      Array.prototype.forEach.call(document.querySelectorAll('.dendro-time-machine, #hub-time-machine'), augmentTimeMachine);
      paintActiveSlice(getPersistedWindow(), latestPayload());
    } catch (err) {}
  }
  var PRESET_ORDER = ['today', '7d', '30d', 'this_month', 'last_month', 'all'];

  function readFact(counts, key) {
    if (!counts || typeof counts !== 'object') return 0;
    var value = counts[key];
    if (typeof value === 'number' && isFinite(value)) return value;
    if (value && typeof value === 'object' && typeof value.rows === 'number') return value.rows;
    return 0;
  }

  function seriesFromPayload(payload) {
    var buckets = payload && (payload.time_buckets || payload.buckets);
    if (Array.isArray(buckets) && buckets.length) {
      return buckets.map(function (bucket) {
        var src = (bucket && (bucket.layers || bucket.counts)) || bucket || {};
        return {
          model: num(src.model != null ? src.model : src.usage_facts),
          harness: num(src.harness != null ? src.harness : src.tool_facts),
          coord: num(src.coord != null ? src.coord : (src.coordination != null ? src.coordination : src.activity_facts)),
          env: num(src.env != null ? src.env : (src.environment != null ? src.environment : src.security_facts))
        };
      });
    }
    var counts = payload && payload.table_counts;
    var model = readFact(counts, 'usage_facts') + readFact(counts, 'message_facts');
    var harness = readFact(counts, 'tool_facts') + readFact(counts, 'skill_facts');
    var coord = readFact(counts, 'activity_facts') + readFact(counts, 'queue_facts') + readFact(counts, 'session_edges');
    var env = readFact(counts, 'security_facts');
    var rowCount = payload && Array.isArray(payload.rows) ? payload.rows.length : 0;
    if (!(model + harness + coord + env) && rowCount) coord = rowCount;
    var seed = (model + harness + coord + env) || 1;
    var out = [];
    for (var i = 0; i < 48; i++) {
      var wave = 0.4 + 0.6 * Math.abs(Math.sin((i + seed) * 0.35));
      out.push({
        model: (model || seed) * wave * 0.25,
        harness: (harness || seed) * (0.35 + 0.65 * Math.abs(Math.sin(i * 0.22))),
        coord: (coord || seed) * (0.4 + 0.6 * Math.abs(Math.cos(i * 0.18))),
        env: (env || 1) * (0.3 + 0.7 * Math.abs(Math.sin(i * 0.27)))
      });
    }
    return out;
  }

  function slicePayload(payload, start, end) {
    var copy;
    try { copy = JSON.parse(JSON.stringify(payload)); }
    catch (err) { copy = Object.assign({}, payload); }
    ['rows', 'seats', 'pairs', 'missions', 'records'].forEach(function (key) {
      if (!Array.isArray(copy[key]) || !copy[key].length) return;
      var list = copy[key];
      var from = Math.max(0, Math.floor(start * list.length));
      var to = Math.min(list.length, Math.ceil(end * list.length));
      if (to <= from) to = Math.min(list.length, from + 1);
      copy[key] = list.slice(from, to);
    });
    return copy;
  }

  function mountTimeMachineScrubber(container, opts) {
    scrubberOpts = opts || scrubberOpts || {};
    if (typeof document === 'undefined' || typeof document.createElement !== 'function') return null;
    if (typeof document.querySelector === 'function') {
      var existing = document.querySelector('.dendro-time-machine, .time-machine-bar, #hub-time-machine');
      if (existing) {
        try { augmentTimeMachine(existing); } catch (err) {}
        return existing;
      }
    }
    var host = container;
    if (!host && typeof document.querySelector === 'function') {
      host = document.querySelector('.main-wrapper') || document.querySelector('main.telemetry-layout') || document.querySelector('main');
    }
    if (!host || typeof host.insertBefore !== 'function') return null;

    var activePreset = getPersistedWindow();
    var initialBounds = rangeForPreset(activePreset);
    var brushStart = initialBounds[0];
    var brushEnd = initialBounds[1];
    var localQueued = false;
    var fetchTimer = null;
    var preferServerPayload = false;

    var root = document.createElement('section');
    root.className = 'dendro-time-machine glass';
    root.setAttribute('aria-label', 'Time Machine');

    var head = document.createElement('div');
    head.className = 'tm-head';
    var title = document.createElement('div');
    title.className = 'tm-title';
    title.textContent = 'Time Machine';
    head.appendChild(title);
    var slicePill = document.createElement('p');
    slicePill.id = 'dendro-active-slice';
    slicePill.className = 'dendro-active-slice';
    slicePill.textContent = activeSliceText(activePreset, null);
    head.appendChild(slicePill);
    var readout = document.createElement('p');
    readout.className = 'tm-window-readout';
    head.appendChild(readout);
    var viewForConcept = scrubberOpts.viewId;
    if (CONCEPTS[viewForConcept]) {
      var how = document.createElement('button');
      how.type = 'button';
      how.className = 'tm-concept-btn';
      how.setAttribute('data-concept', viewForConcept);
      how.textContent = 'How to read this';
      head.appendChild(how);
    }
    root.appendChild(head);

    var seg = document.createElement('div');
    seg.className = 'tm-seg-control';
    seg.setAttribute('role', 'tablist');
    seg.style.setProperty('--tm-index', String(Math.max(0, PRESET_ORDER.indexOf(activePreset))));
    if (splitRange(activePreset)) seg.classList.add('tm-custom-active');
    var pill = document.createElement('span');
    pill.className = 'tm-seg-pill';
    pill.setAttribute('aria-hidden', 'true');
    seg.appendChild(pill);
    var presetButtons = {};
    var labels = {
      today: 'Today (24h)',
      '7d': 'Last Week (7d)',
      '30d': '30 Days',
      this_month: 'This Month',
      last_month: 'Last Month',
      all: 'All Time'
    };
    PRESET_ORDER.forEach(function (id, index) {
      var button = document.createElement('button');
      button.type = 'button';
      button.textContent = labels[id];
      button.setAttribute('data-preset', id);
      button.setAttribute('data-window', id);
      button.classList.add('tm-preset');
      button.setAttribute('aria-pressed', id === activePreset ? 'true' : 'false');
      button.addEventListener('click', function () { applyPreset(id, false); });
      presetButtons[id] = { button: button, index: index };
      seg.appendChild(button);
    });
    var segRow = document.createElement('div');
    segRow.className = 'tm-seg-row';
    var customBtn = document.createElement('button');
    customBtn.type = 'button';
    customBtn.className = 'tm-custom-range';
    customBtn.setAttribute('data-window', 'custom');
    customBtn.setAttribute('data-preset', 'custom');
    customBtn.setAttribute('aria-haspopup', 'dialog');
    customBtn.textContent = splitRange(activePreset)
      ? ('Custom (' + formatHumanWindow(splitRange(activePreset)[0], splitRange(activePreset)[1], activePreset) + ')')
      : 'Custom Range ▾';
    customBtn.setAttribute('aria-pressed', splitRange(activePreset) ? 'true' : 'false');
    segRow.appendChild(seg);
    segRow.appendChild(customBtn);
    root.appendChild(segRow);

    var wrap = document.createElement('div');
    wrap.className = 'tm-canvas-wrap';
    var canvas = document.createElement('canvas');
    canvas.setAttribute('aria-hidden', 'true');
    var pearls = document.createElement('div');
    pearls.className = 'tm-pearls';
    var brush = document.createElement('div');
    brush.className = 'tm-brush';
    var startHandle = document.createElement('button');
    startHandle.type = 'button';
    startHandle.className = 'tm-brush-handle tm-brush-start';
    startHandle.setAttribute('aria-label', 'Window start');
    var endHandle = document.createElement('button');
    endHandle.type = 'button';
    endHandle.className = 'tm-brush-handle tm-brush-end';
    endHandle.setAttribute('aria-label', 'Window end');
    brush.appendChild(startHandle);
    brush.appendChild(endHandle);
    wrap.appendChild(canvas);
    wrap.appendChild(pearls);
    wrap.appendChild(brush);
    root.appendChild(wrap);
    var axis = document.createElement('div');
    axis.className = 'tm-axis';
    axis.setAttribute('aria-hidden', 'true');
    var brushApply = document.createElement('button');
    brushApply.type = 'button';
    brushApply.className = 'tm-brush-apply';
    brushApply.hidden = true;
    root.appendChild(axis);
    root.appendChild(brushApply);
    host.insertBefore(root, host.firstChild || null);

    function positionBrush() {
      brush.style.left = (brushStart * 100) + '%';
      brush.style.width = ((brushEnd - brushStart) * 100) + '%';
    }

    function canonicalPayload() {
      var state = DendroState.get();
      if (state && state.lastPayload) return state.lastPayload;
      try {
        if (scrubberOpts.loadEmbedded) return scrubberOpts.loadEmbedded();
      } catch (err) {}
      return null;
    }

    function filteredFromBrush() {
      var base = canonicalPayload();
      if (!base || typeof base !== 'object') return null;
      if (preferServerPayload) return normalizeViewPayload(base, null, scrubberOpts.viewId || '');
      var sliced = slicePayload(base, brushStart, brushEnd);
      return normalizeViewPayload(sliced, null, scrubberOpts.viewId || '');
    }

    function scheduleLocal() {
      if (localQueued) return;
      localQueued = true;
      var run = function () {
        localQueued = false;
        var payload = filteredFromBrush();
        if (payload && scrubberOpts.emitLocal) scrubberOpts.emitLocal(payload);
      };
      if (typeof requestAnimationFrame === 'function') requestAnimationFrame(run);
      else run();
    }

    function fetchWindow(preset) {
      if (typeof fetch !== 'function') {
        preferServerPayload = false;
        scheduleLocal();
        return;
      }
      var generation = ++scrubberFetchGen;
      var view = scrubberOpts.viewId || 'hub';
      var base = scrubberOpts.pollEndpoint || '/api/query';
      var url = base + '?view=' + encodeURIComponent(view) + '&window=' + encodeURIComponent(preset);
      fetch(url, { cache: 'no-store' }).then(function (res) {
        if (!res || !res.ok) throw new Error('window fetch failed');
        return res.json();
      }).then(function (data) {
        if (generation !== scrubberFetchGen || data == null) return;
        preferServerPayload = true;
        if (scrubberOpts.publishLive) scrubberOpts.publishLive(data);
      }).catch(function () {
        if (generation !== scrubberFetchGen) return;
        preferServerPayload = false;
        scheduleLocal();
      });
    }

    function applyPreset(id, fromState) {
      if (!PRESET_WINDOWS[id] && !splitRange(id)) id = 'all';
      activePreset = id;
      var bounds = rangeForPreset(id);
      brushStart = bounds[0];
      brushEnd = bounds[1];
      var meta = presetButtons[id];
      var custom = !!splitRange(id);
      if (custom) seg.classList.add('tm-custom-active');
      else {
        seg.classList.remove('tm-custom-active');
        seg.style.setProperty('--tm-index', String(meta ? meta.index : Math.max(0, PRESET_ORDER.indexOf(id))));
      }
      Object.keys(presetButtons).forEach(function (key) {
        presetButtons[key].button.setAttribute('aria-pressed', !custom && key === id ? 'true' : 'false');
      });
      if (customBtn) {
        customBtn.setAttribute('aria-pressed', custom ? 'true' : 'false');
        var parts = splitRange(id);
        customBtn.textContent = parts
          ? ('Custom (' + formatHumanWindow(parts[0], parts[1], id) + ')')
          : 'Custom Range ▾';
      }
      if (brushApply) brushApply.hidden = true;
      positionBrush();
      if (!fromState) {
        DendroState.set({ preset: id, range: { start: brushStart, end: brushEnd } });
      } else {
        try { setPersistedWindow(id); } catch (err) {}
      }
      try { paintActiveSlice(id, canonicalPayload()); } catch (err) {}
      fetchWindow(id);
    }

    function paintSoon() {
      if (typeof requestAnimationFrame === 'function') requestAnimationFrame(draw);
      else draw();
    }

    function draw() {
      var payload = canonicalPayload();
      var series = seriesFromPayload(payload);
      var width = wrap.clientWidth || 640;
      var height = wrap.clientHeight || 92;
      var dpr = Math.max(1, (typeof window !== 'undefined' && window.devicePixelRatio) || 1);
      canvas.width = Math.floor(width * dpr);
      canvas.height = Math.floor(height * dpr);
      canvas.style.width = width + 'px';
      canvas.style.height = height + 'px';
      var ctx = canvas.getContext && canvas.getContext('2d');
      if (!ctx) return;
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      ctx.clearRect(0, 0, width, height);
      var max = 1;
      series.forEach(function (bucket) {
        max = Math.max(max, bucket.model + bucket.harness + bucket.coord + bucket.env);
      });
      var slot = width / series.length;
      series.forEach(function (bucket, index) {
        var x = index * slot;
        var layers = [
          ['#8b5cf6', bucket.env],
          ['#10b981', bucket.coord],
          ['#06b6d4', bucket.harness],
          ['#F2A541', bucket.model]
        ];
        var y = height - 8;
        layers.forEach(function (layer) {
          var h = (layer[1] / max) * (height - 16);
          y -= h;
          ctx.fillStyle = layer[0];
          ctx.globalAlpha = 0.9;
          ctx.fillRect(x + 1, y, Math.max(1, slot - 2), h);
        });
      });
      ctx.globalAlpha = 1;
      pearls.replaceChildren ? pearls.replaceChildren() : (pearls.innerHTML = '');
      var anomalies = [];
      if (payload && Array.isArray(payload.anomalies)) anomalies = payload.anomalies;
      else if (payload && payload.anomaly_spotlights && Array.isArray(payload.anomaly_spotlights.findings)) anomalies = payload.anomaly_spotlights.findings;
      anomalies.slice(0, 12).forEach(function (anomaly, index) {
        var at = anomaly && typeof anomaly.at === 'number' ? anomaly.at : (anomaly && typeof anomaly.t === 'number' ? anomaly.t : ((index + 0.5) / Math.max(anomalies.length, 1)));
        var pearl = document.createElement('button');
        pearl.type = 'button';
        pearl.className = 'tm-pearl';
        pearl.style.left = (Math.max(0, Math.min(1, at)) * 100) + '%';
        pearl.style.top = '28%';
        pearl.style.setProperty('--pearl', anomaly && anomaly.layer === 'model' ? '#F2A541' : '#f43f5e');
        pearl.setAttribute('aria-label', (anomaly && (anomaly.label || anomaly.title)) || 'Anomaly');
        pearl.addEventListener('click', function () {
          DendroState.set({ hover: anomaly || { at: at }, focus: 'anomaly' });
        });
        pearls.appendChild(pearl);
      });
    }

    function bindHandle(handle, edge) {
      if (!handle.addEventListener) return;
      handle.addEventListener('pointerdown', function (event) {
        if (event.preventDefault) event.preventDefault();
        function ratio(ev) {
          var rect = wrap.getBoundingClientRect();
          if (!rect || !rect.width) return edge === 'start' ? brushStart : brushEnd;
          return (ev.clientX - rect.left) / rect.width;
        }
        function move(ev) {
          var value = ratio(ev);
          if (edge === 'start') brushStart = Math.max(0, Math.min(value, brushEnd - 0.02));
          else brushEnd = Math.min(1, Math.max(value, brushStart + 0.02));
          positionBrush();
          preferServerPayload = false;
          DendroState.set({ preset: activePreset, range: { start: brushStart, end: brushEnd } });
          scheduleLocal();
          if (fetchTimer && typeof clearTimeout === 'function') clearTimeout(fetchTimer);
          if (typeof setTimeout === 'function') fetchTimer = setTimeout(function () { fetchWindow(activePreset); }, 180);
        }
        function up() {
          document.removeEventListener('pointermove', move);
          document.removeEventListener('pointerup', up);
          if (fetchTimer && typeof clearTimeout === 'function') clearTimeout(fetchTimer);
          fetchTimer = null;
          showBrushedApply(root, brushStart, brushEnd);
        }
        document.addEventListener('pointermove', move);
        document.addEventListener('pointerup', up);
      });
    }
    bindHandle(startHandle, 'start');
    bindHandle(endHandle, 'end');
    positionBrush();

    DendroState.subscribe(function (state) {
      if (state.preset && state.preset !== activePreset && (PRESET_WINDOWS[state.preset] || splitRange(state.preset))) {
        applyPreset(state.preset, true);
        return;
      }
      paintSoon();
    });
    paintSoon();
    try { paintActiveSlice(activePreset, canonicalPayload()); } catch (err) {}
    return root;
  }

  var CONCEPTS = {
    trajectory_vortex: {
      title: 'Trajectory vortex',
      kicker: 'For everyone',
      plain: 'A vortex is a loop: the same work keeps coming back instead of moving forward. The ring tightens when a path repeats more than the tripwire allows.',
      formula: 'vortex_loops = count(cycles where the next hop returns to a prior actor)\nTripwire: vortex_loops > 0 on a single task path.\nA loop is evidence of churn, not a verdict about who caused it.',
      svg: '<svg viewBox="0 0 640 240" role="img" aria-label="Looping trajectory"><path class="sap-pulse" d="M80 120 C 160 20, 280 20, 320 120 S 480 220, 560 120" fill="none" stroke="#06b6d4" stroke-width="3"/><circle cx="320" cy="120" r="28" fill="none" stroke="#f43f5e" stroke-width="3"/><circle cx="80" cy="120" r="6" fill="#F2A541"/><circle cx="560" cy="120" r="6" fill="#10b981"/></svg>'
    },
    schema_drift: {
      title: 'Schema drift',
      kicker: 'For everyone',
      plain: 'Drift means the shape of a record changed: a field appeared, disappeared, or started meaning something else. The stacked bars slide out of alignment when the shape no longer matches the last agreed cut.',
      formula: 'drift = fields(current) symmetric_difference fields(baseline)\nTripwire: any required field missing, or a new field on more than one source in the window.\nDrift is a shape change, not proof that a number is wrong.',
      svg: '<svg viewBox="0 0 640 240" role="img" aria-label="Sliding schema layers"><rect x="70" y="48" width="220" height="28" rx="6" fill="#06b6d4"/><rect x="70" y="88" width="180" height="28" rx="6" fill="#10b981"/><rect x="300" y="48" width="220" height="28" rx="6" fill="#06b6d4"/><rect x="340" y="88" width="180" height="28" rx="6" fill="#F2A541"/><path class="sap-pulse" d="M300 62 H340" stroke="#f43f5e" stroke-width="2"/></svg>'
    },
    authority_blast_radius: {
      title: 'Authority blast radius',
      kicker: 'For everyone',
      plain: 'Blast radius is how far one actor\'s effective authority can reach: which scopes, tools, and seats a single grant can touch if it is misused or copied.',
      formula: 'radius = count(distinct scopes reachable from actor authority)\nTripwire: a grant that crosses a scope boundary, or effective_authority wider than the seat\'s declared role.\nReach is exposure, not evidence of misuse.',
      svg: '<svg viewBox="0 0 640 240" role="img" aria-label="Authority ripple"><circle cx="220" cy="120" r="18" fill="#8b5cf6"/><circle class="sap-pulse" cx="220" cy="120" r="48" fill="none" stroke="#8b5cf6"/><circle class="sap-pulse" cx="220" cy="120" r="84" fill="none" stroke="#06b6d4"/><circle cx="420" cy="70" r="8" fill="#F2A541"/><circle cx="460" cy="150" r="8" fill="#10b981"/></svg>'
    },
    counterfactual_routing: {
      title: 'Counterfactual routing',
      kicker: 'For everyone',
      plain: 'A counterfactual asks what would have happened on another route. It compares the path that ran with a path that was available, without claiming the other path was better.',
      formula: 'delta = cost(chosen route) - cost(alternate route)\nTripwire: |delta| above the window median, with both routes observed.\nThe alternate is a comparison, not an effect.',
      svg: '<svg viewBox="0 0 640 240" role="img" aria-label="Two routes"><path d="M40 160 H240" stroke="#10b981" stroke-width="4"/><path d="M40 160 C 140 40, 220 40, 280 160 H600" fill="none" stroke="#F2A541" stroke-width="3" stroke-dasharray="6 6"/><circle cx="240" cy="160" r="7" fill="#10b981"/></svg>'
    },
    seat_dynamics: {
      title: 'Actor activity',
      kicker: 'For everyone',
      plain: 'Each band is one actor\'s time: active work, idle while work was waiting, container hold, stall, and idle with nothing queued. The chart disappears only when there is no actor evidence at all.',
      formula: 'active_pct = active_secs / observed_secs × 100\nidle_work_pct = idle_with_work_secs / observed_secs × 100\nTripwire: idle-with-work or unparked hold that dominates observed time.',
      svg: '<svg viewBox="0 0 640 240" role="img" aria-label="Activity bands"><rect x="40" y="70" width="360" height="22" fill="#10b981"/><rect x="400" y="70" width="80" height="22" fill="#F2A541"/><rect x="480" y="70" width="40" height="22" fill="#f43f5e"/><rect x="520" y="70" width="80" height="22" fill="#64748b"/><rect x="40" y="120" width="220" height="22" fill="#10b981"/><rect x="260" y="120" width="160" height="22" fill="#F2A541"/></svg>'
    },
    handoff_flow: {
      title: 'Handoff flow',
      kicker: 'For everyone',
      plain: 'Every cell is a handoff from one actor to another. Darker cells mean more returns of the same work. The heatmap needs pairs; when the live window only sends rows, those pairs are rebuilt so the grid stays up.',
      formula: 'return_rate = rework_returns / handoffs\nfleet_return_rate = sum(rework_returns) / sum(handoffs)\nTripwire: return_rate above the fleet rate, or a ring-boundary crossing.',
      svg: '<svg viewBox="0 0 640 240" role="img" aria-label="Handoff grid"><g fill="#10b981"><rect x="80" y="40" width="48" height="48"/><rect x="136" y="96" width="48" height="48" opacity="0.7"/></g><g fill="#f43f5e"><rect x="136" y="40" width="48" height="48" opacity="0.85"/><rect x="80" y="96" width="48" height="48" opacity="0.45"/></g></svg>'
    },
    cost_and_loops: {
      title: 'Task and actor token burn',
      kicker: 'For everyone',
      plain: 'Burn splits tokens into fresh input, cache read, cache write, and output. Cache is the reused portion. A mission row is one actor, model, or session in the window.',
      formula: 'total_tokens = input_uncached + cache_read + cache_write + output\ncache_read_ratio = cache_read / (input_uncached + cache_read + cache_write)\nTripwire: cache_read_ratio collapsing while fresh input climbs.',
      svg: '<svg viewBox="0 0 640 240" role="img" aria-label="Token split"><rect x="60" y="80" width="80" height="80" fill="#06b6d4"/><rect x="150" y="40" width="220" height="120" fill="#10b981"/><rect x="380" y="100" width="60" height="60" fill="#8b5cf6"/><rect x="450" y="70" width="120" height="90" fill="#F2A541"/></svg>'
    },
    executive_governance: {
      title: 'Governance scorecard',
      kicker: 'For everyone',
      plain: 'The scorecard rolls peer review, watchdog recovery, vortex loops, yield, and freshness into one window. A record is one scope\'s evidence, not a permission to act.',
      formula: 'peer_review passes when four_eyes_independence.compliant is true\nTripwire: vortex_loops > 0, or peer review not compliant, or fresh = false.',
      svg: '<svg viewBox="0 0 640 240" role="img" aria-label="Scorecard shield"><polygon points="320,24 460,90 420,190 220,190 180,90" fill="none" stroke="#10b981" stroke-width="3"/><circle cx="320" cy="120" r="8" fill="#10b981"/></svg>'
    },
    message_mix: {
      title: 'Message mix',
      kicker: 'For everyone',
      plain: 'Message mix counts who spoke, in which direction, and through which modality. It never stores message bodies. The mix shows shape, not the words.',
      formula: 'share(role, direction, modality) = count / messages in the window\nTripwire: one direction or modality above 90% of the window, which usually means a missing feed.',
      svg: '<svg viewBox="0 0 640 240" role="img" aria-label="Message mix bars"><rect x="80" y="40" width="36" height="150" fill="#F2A541"/><rect x="140" y="80" width="36" height="110" fill="#06b6d4"/><rect x="200" y="110" width="36" height="80" fill="#10b981"/><rect x="260" y="60" width="36" height="130" fill="#8b5cf6"/></svg>'
    },
    subagent_lineage: {
      title: 'Subagent lineage',
      kicker: 'For everyone',
      plain: 'Lineage is the parent-to-child session tree. A wide tree means many helpers. A broken edge means a child started without a finish, or a finish without a start.',
      formula: 'fanout = distinct child sessions / parent\nTripwire: failed_count > 0, or started_count far above finished_count.',
      svg: '<svg viewBox="0 0 640 240" role="img" aria-label="Session tree"><circle cx="320" cy="40" r="10" fill="#10b981"/><path d="M320 50 L180 120 M320 50 L320 120 M320 50 L460 120" stroke="#06b6d4" fill="none"/><circle cx="180" cy="130" r="8" fill="#F2A541"/><circle cx="320" cy="130" r="8" fill="#10b981"/><circle cx="460" cy="130" r="8" fill="#f43f5e"/></svg>'
    },
    security_posture: {
      title: 'Security posture',
      kicker: 'For everyone',
      plain: 'Posture lists the kinds of authority and sensitive fields that were observed. It is a count of evidence types, not a scan of secret values.',
      formula: 'posture(kind) = count of observations of that kind in the window\nTripwire: a kind whose value crosses a scope, or effective authority outside the seat role.',
      svg: '<svg viewBox="0 0 640 240" role="img" aria-label="Posture lock"><rect x="250" y="90" width="140" height="100" rx="12" fill="none" stroke="#8b5cf6" stroke-width="3"/><path d="M280 90 V70 a40 40 0 0 1 80 0 V90" fill="none" stroke="#06b6d4" stroke-width="3"/></svg>'
    },
    process_resources: {
      title: 'Process resources',
      kicker: 'For everyone',
      plain: 'Resource rows attribute memory to a process. Bytes stay bytes. A row with an unknown unit is shown as insufficient evidence rather than converted.',
      formula: 'rss is reported only when rss_unit = bytes\nTripwire: mixing byte and non-byte units in one total.\nUnits are never mixed.',
      svg: '<svg viewBox="0 0 640 240" role="img" aria-label="Memory bars"><rect x="70" y="140" width="80" height="50" fill="#06b6d4"/><rect x="170" y="90" width="80" height="100" fill="#10b981"/><rect x="270" y="50" width="80" height="140" fill="#F2A541"/><rect x="370" y="110" width="80" height="80" fill="#8b5cf6"/></svg>'
    }
  };

  var conceptRoot = null;

  function liveProofLine(conceptId) {
    var payload = DendroState.get().lastPayload;
    if (!payload || typeof payload !== 'object') return 'Live count: the current window has not loaded yet.';
    var count = 0;
    if (conceptId === 'seat_dynamics') count = (payload.seats || payload.rows || []).length;
    else if (conceptId === 'handoff_flow') count = (payload.pairs || payload.rows || []).length;
    else if (conceptId === 'cost_and_loops') count = (payload.missions || payload.rows || []).length;
    else if (conceptId === 'executive_governance') count = (payload.records || payload.rows || []).length;
    else if (Array.isArray(payload.rows)) count = payload.rows.length;
    else count = readFact(payload.table_counts, 'usage_facts') + readFact(payload.table_counts, 'activity_facts');
    var preset = DendroState.get().preset || 'all';
    return 'Live count in the ' + preset + ' telemetry window: ' + count + '.';
  }

  function ensureConceptSheet() {
    if (conceptRoot) return conceptRoot;
    if (typeof document === 'undefined' || typeof document.createElement !== 'function' || !document.body) return null;
    conceptRoot = document.createElement('div');
    conceptRoot.className = 'dendro-concept-sheet';
    conceptRoot.hidden = true;
    conceptRoot.setAttribute('role', 'dialog');
    conceptRoot.setAttribute('aria-modal', 'true');
    var card = document.createElement('div');
    card.className = 'concept-sheet-card glass';
    card.innerHTML = '<div class="concept-sheet-top"><div><div class="concept-kicker"></div><h2></h2></div><button type="button" class="concept-close">Close</button></div><div class="concept-tabs" role="tablist"><button type="button" data-tab="intuition" aria-selected="true">Visual Intuition (For Everyone)</button><button type="button" data-tab="formula" aria-selected="false">Formula, Thresholds & Live Proof (For Power Users)</button></div><div class="concept-panel" data-panel="intuition"><div class="concept-svg-stage"></div><p class="concept-copy"></p></div><div class="concept-panel" data-panel="formula" hidden><div class="concept-formula-box"></div><p class="concept-live-proof"></p></div>';
    conceptRoot.appendChild(card);
    document.body.appendChild(conceptRoot);
    card.querySelector('.concept-close').addEventListener('click', closeConceptSheet);
    conceptRoot.addEventListener('click', function (event) {
      if (event.target === conceptRoot) closeConceptSheet();
    });
    Array.prototype.forEach.call(card.querySelectorAll('.concept-tabs button'), function (button) {
      button.addEventListener('click', function () {
        var tab = button.getAttribute('data-tab');
        Array.prototype.forEach.call(card.querySelectorAll('.concept-tabs button'), function (item) {
          item.setAttribute('aria-selected', item === button ? 'true' : 'false');
        });
        Array.prototype.forEach.call(card.querySelectorAll('.concept-panel'), function (panel) {
          panel.hidden = panel.getAttribute('data-panel') !== tab;
        });
      });
    });
    return conceptRoot;
  }

  function closeConceptSheet() {
    if (conceptRoot) conceptRoot.hidden = true;
  }

  function openConceptSheet(conceptId) {
    var sheet = ensureConceptSheet();
    if (!sheet) return;
    var concept = CONCEPTS[conceptId] || {
      title: 'Primer',
      kicker: 'For everyone',
      plain: 'This view does not have a visual primer yet.',
      formula: 'No formula is registered for this id.',
      svg: '<svg viewBox="0 0 640 220" role="img" aria-label="Empty primer"></svg>'
    };
    sheet.querySelector('.concept-kicker').textContent = concept.kicker || 'Primer';
    sheet.querySelector('h2').textContent = concept.title;
    sheet.querySelector('.concept-svg-stage').innerHTML = concept.svg;
    sheet.querySelector('.concept-copy').textContent = concept.plain;
    sheet.querySelector('.concept-formula-box').textContent = concept.formula;
    sheet.querySelector('.concept-live-proof').textContent = liveProofLine(conceptId);
    var card = sheet.querySelector('.concept-sheet-card');
    Array.prototype.forEach.call(card.querySelectorAll('.concept-tabs button'), function (item) {
      item.setAttribute('aria-selected', item.getAttribute('data-tab') === 'intuition' ? 'true' : 'false');
    });
    Array.prototype.forEach.call(card.querySelectorAll('.concept-panel'), function (panel) {
      panel.hidden = panel.getAttribute('data-panel') !== 'intuition';
    });
    sheet.hidden = false;
    DendroState.set({ focus: conceptId });
  }

  var spotlightEl = null;
  var spotlightInput = null;
  var spotlightList = null;
  var spotlightRings = null;
  var spotlightIndex = 0;

  var SPOTLIGHT_COMMANDS = [
    { kind: 'Time', label: 'Today', terms: 'today 24h', run: function () { DendroState.set({ preset: 'today' }); } },
    { kind: 'Time', label: 'Last week', terms: 'last week 7d', run: function () { DendroState.set({ preset: '7d' }); } },
    { kind: 'Time', label: '30 days', terms: '30 days 30d', run: function () { DendroState.set({ preset: '30d' }); } },
    { kind: 'Time', label: 'This month', terms: 'this month', run: function () { DendroState.set({ preset: 'this_month' }); } },
    { kind: 'Time', label: 'Last month', terms: 'last month', run: function () { DendroState.set({ preset: 'last_month' }); } },
    { kind: 'Time', label: 'All time', terms: 'all time', run: function () { DendroState.set({ preset: 'all' }); } },
    { kind: 'Concept', label: 'Trajectory vortex', terms: 'vortex trajectory', run: function () { openConceptSheet('trajectory_vortex'); } },
    { kind: 'Concept', label: 'Schema drift', terms: 'drift schema', run: function () { openConceptSheet('schema_drift'); } },
    { kind: 'Concept', label: 'Authority blast radius', terms: 'blast radius authority', run: function () { openConceptSheet('authority_blast_radius'); } },
    { kind: 'Concept', label: 'Cache and token burn', terms: 'cache cost tokens', run: function () { openConceptSheet('cost_and_loops'); } },
    { kind: 'Layer', label: 'Model layer', terms: 'model', run: function () { activateLayer('model'); } },
    { kind: 'Layer', label: 'Harness layer', terms: 'harness', run: function () { activateLayer('harness'); } },
    { kind: 'Layer', label: 'Coordination layer', terms: 'coordination coord', run: function () { activateLayer('coordination'); } },
    { kind: 'Layer', label: 'Environment layer', terms: 'environment env', run: function () { activateLayer('environment'); } },
    { kind: 'Console', label: 'Hub', terms: 'hub home', href: 'hub' },
    { kind: 'Console', label: 'Actor activity', terms: 'seat dynamics actors', href: 'seat_dynamics' },
    { kind: 'Console', label: 'Handoff flow', terms: 'handoff heatmap', href: 'handoff_flow' },
    { kind: 'Console', label: 'Token burn', terms: 'cost loops cache', href: 'cost_and_loops' },
    { kind: 'Console', label: 'Governance', terms: 'executive governance', href: 'executive_governance' },
    { kind: 'Console', label: 'Message mix', terms: 'message mix', href: 'message_mix' },
    { kind: 'Console', label: 'Subagent lineage', terms: 'subagent lineage', href: 'subagent_lineage' },
    { kind: 'Console', label: 'Security posture', terms: 'security posture', href: 'security_posture' },
    { kind: 'Console', label: 'Process resources', terms: 'process resources memory', href: 'process_resources' },
    { kind: 'Console', label: 'Trajectory vortex console', terms: 'vortex console', href: 'trajectory_vortex' },
    { kind: 'Console', label: 'Schema drift console', terms: 'drift console', href: 'schema_drift' },
    { kind: 'Console', label: 'Blast radius console', terms: 'blast console', href: 'authority_blast_radius' },
    { kind: 'Console', label: 'Counterfactual routing', terms: 'counterfactual routing', href: 'counterfactual_routing' }
  ];

  function consoleHref(name) {
    var path = (window.location && window.location.pathname) || '';
    var inViews = path.indexOf('/views/') !== -1;
    if (name === 'hub') return inViews ? '../index.html' : 'index.html';
    return inViews ? (name + '.html') : ('views/' + name + '.html');
  }

  function activateLayer(layer) {
    DendroState.set({ layer: layer });
    if (typeof document.querySelector !== 'function') return;
    var chip = document.querySelector('.filter-chip[data-category="' + layer + '"]');
    if (chip && typeof chip.click === 'function') chip.click();
  }

  function spotlightMatches(query) {
    var q = (query || '').trim().toLowerCase();
    if (!q) return SPOTLIGHT_COMMANDS.slice(0, 8);
    return SPOTLIGHT_COMMANDS.filter(function (command) {
      return (command.label + ' ' + command.terms + ' ' + command.kind).toLowerCase().indexOf(q) !== -1;
    });
  }

  function paintMiniRings(query) {
    if (!spotlightRings) return;
    var q = (query || '').toLowerCase();
    Array.prototype.forEach.call(spotlightRings.querySelectorAll('circle'), function (circle) {
      var layer = circle.getAttribute('data-layer') || '';
      var hit = !q || q.indexOf(layer) !== -1 || layer.indexOf(q) !== -1;
      circle.style.opacity = hit ? '1' : '0.28';
    });
  }

  function renderSpotlight(query) {
    if (!spotlightList) return;
    var matches = spotlightMatches(query);
    if (spotlightIndex >= matches.length) spotlightIndex = 0;
    spotlightList.replaceChildren ? spotlightList.replaceChildren() : (spotlightList.innerHTML = '');
    matches.forEach(function (command, index) {
      var item = document.createElement('li');
      var button = document.createElement('button');
      button.type = 'button';
      button.setAttribute('aria-selected', index === spotlightIndex ? 'true' : 'false');
      var name = document.createElement('span');
      name.textContent = command.label;
      var kind = document.createElement('span');
      kind.className = 'spotlight-kind';
      kind.textContent = command.kind;
      button.appendChild(name);
      button.appendChild(kind);
      button.addEventListener('click', function () { runSpotlight(command); });
      item.appendChild(button);
      spotlightList.appendChild(item);
    });
    paintMiniRings(query);
    spotlightList._matches = matches;
  }

  function runSpotlight(command) {
    if (!command) return;
    closeSpotlight();
    if (command.href) {
      window.location.href = consoleHref(command.href);
      return;
    }
    if (command.run) command.run();
  }

  function closeSpotlight() {
    if (!spotlightEl) return;
    if (typeof spotlightEl.close === 'function' && spotlightEl.open) spotlightEl.close();
    spotlightEl.hidden = true;
  }

  function ensureSpotlight() {
    if (spotlightEl) return spotlightEl;
    if (typeof document === 'undefined' || typeof document.createElement !== 'function' || !document.body) return null;
    var dialog = document.createElement('dialog');
    dialog.className = 'dendro-spotlight';
    var panel = document.createElement('div');
    panel.className = 'spotlight-panel glass';
    spotlightInput = document.createElement('input');
    spotlightInput.className = 'spotlight-input';
    spotlightInput.type = 'text';
    spotlightInput.placeholder = 'Time, concept, layer, or console';
    spotlightInput.setAttribute('aria-label', 'Spotlight');
    spotlightList = document.createElement('ul');
    spotlightList.className = 'spotlight-results';
    spotlightList.setAttribute('role', 'listbox');
    spotlightRings = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
    spotlightRings.setAttribute('class', 'spotlight-mini-rings');
    spotlightRings.setAttribute('viewBox', '0 0 120 120');
    spotlightRings.setAttribute('width', '120');
    spotlightRings.setAttribute('height', '120');
    spotlightRings.setAttribute('aria-hidden', 'true');
    [['env', '52', '#8b5cf6'], ['coordination', '40', '#10b981'], ['harness', '28', '#06b6d4'], ['model', '16', '#F2A541']].forEach(function (spec) {
      var circle = document.createElementNS('http://www.w3.org/2000/svg', 'circle');
      circle.setAttribute('cx', '60');
      circle.setAttribute('cy', '60');
      circle.setAttribute('r', spec[1]);
      circle.setAttribute('stroke', spec[2]);
      circle.setAttribute('data-layer', spec[0]);
      spotlightRings.appendChild(circle);
    });
    panel.appendChild(spotlightInput);
    panel.appendChild(spotlightList);
    panel.appendChild(spotlightRings);
    dialog.appendChild(panel);
    document.body.appendChild(dialog);
    spotlightEl = dialog;
    spotlightInput.addEventListener('input', function () {
      spotlightIndex = 0;
      renderSpotlight(spotlightInput.value);
    });
    spotlightInput.addEventListener('keydown', function (event) {
      var matches = spotlightList._matches || [];
      if (event.key === 'ArrowDown') {
        spotlightIndex = Math.min(matches.length - 1, spotlightIndex + 1);
        renderSpotlight(spotlightInput.value);
        event.preventDefault();
      } else if (event.key === 'ArrowUp') {
        spotlightIndex = Math.max(0, spotlightIndex - 1);
        renderSpotlight(spotlightInput.value);
        event.preventDefault();
      } else if (event.key === 'Enter') {
        runSpotlight(matches[spotlightIndex]);
        event.preventDefault();
      } else if (event.key === 'Escape') {
        closeSpotlight();
      }
    });
    dialog.addEventListener('click', function (event) {
      if (event.target === dialog) closeSpotlight();
    });
    dialog.addEventListener('close', function () { dialog.hidden = true; });
    return dialog;
  }

  function openSpotlight() {
    var dialog = ensureSpotlight();
    if (!dialog) return;
    spotlightIndex = 0;
    if (spotlightInput) spotlightInput.value = '';
    renderSpotlight('');
    dialog.hidden = false;
    if (typeof dialog.showModal === 'function') {
      try { if (!dialog.open) dialog.showModal(); }
      catch (err) {}
    }
    if (spotlightInput && typeof spotlightInput.focus === 'function') spotlightInput.focus();
  }

  function bindConceptClicks() {
    if (typeof document === 'undefined' || typeof document.addEventListener !== 'function') return;
    var root = document.documentElement;
    if (!root || typeof root.getAttribute !== 'function') return;
    if (root.getAttribute('data-dendro-concepts')) return;
    root.setAttribute('data-dendro-concepts', '1');
    document.addEventListener('click', function (event) {
      var node = event && event.target;
      if (!node || typeof node.closest !== 'function') return;
      var trigger = node.closest('[data-concept]');
      if (!trigger) return;
      var id = trigger.getAttribute('data-concept');
      if (!id) return;
      if (event.preventDefault) event.preventDefault();
      openConceptSheet(id);
    });
  }

  function bindGlobalKeys() {
    if (typeof document === 'undefined' || typeof document.addEventListener !== 'function') return;
    var root = document.documentElement;
    if (!root || typeof root.setAttribute !== 'function') return;
    if (root.getAttribute('data-dendro-spotlight-keys')) return;
    root.setAttribute('data-dendro-spotlight-keys', '1');
    document.addEventListener('keydown', function (event) {
      if (!event) return;
      var key = event.key || '';
      if (key === 'Escape') {
        closeConceptSheet();
        closeSpotlight();
        closeCustomRangePopover();
        return;
      }
      var metaK = (event.metaKey || event.ctrlKey) && (key === 'k' || key === 'K');
      var slash = key === '/' && !event.metaKey && !event.ctrlKey && !event.altKey;
      if (!metaK && !slash) return;
      var target = event.target;
      if (!metaK && target && target.tagName) {
        var tag = target.tagName;
        if (tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT' || target.isContentEditable) return;
      }
      if (event.preventDefault) event.preventDefault();
      openSpotlight();
    });
  }

  var DendroUI = {
    openConceptSheet: openConceptSheet,
    openSpotlight: openSpotlight,
    mountTimeMachineScrubber: mountTimeMachineScrubber,
    formatHumanWindow: formatHumanWindow,
    openCustomRangePopover: openCustomRangePopover,
    normalizePreset: normalizePreset
  };
  global.DendroUI = DendroUI;

  function mountProductShell() {
    try {
      if (typeof document === 'undefined' || !document.body) return;
      if (typeof document.createElement !== 'function') return;
      if (typeof document.querySelector === 'function' && document.querySelector('.dendro-sidebar')) return;

      var styleLink = typeof document.querySelector === 'function' ? document.querySelector('link[rel="stylesheet"]') : null;
      var styleHref = styleLink && styleLink.getAttribute ? (styleLink.getAttribute('href') || '') : '';
      var nested = styleHref.indexOf('../style.css') !== -1;
      var dash = nested ? '../' : '';
      var root = nested ? '../../' : '../';
      function viewHref(name) {
        return (nested ? '' : 'views/') + name;
      }

      var path = (typeof window !== 'undefined' && window.location && window.location.pathname) ? window.location.pathname : '';
      var here = path.split('/').pop() || '';

      var groups = [
        { label: 'Overview', items: [
          { href: dash + 'index.html', label: 'Intelligence Hub' },
          { href: root + 'viewer.html', label: 'Fleet Explorer' }
        ]},
        { label: 'Active Intelligence', items: [
          { href: viewHref('trajectory_vortex.html'), label: 'Trajectory Vortex' },
          { href: viewHref('schema_drift.html'), label: 'Schema Drift' },
          { href: viewHref('authority_blast_radius.html'), label: 'Blast Radius' },
          { href: viewHref('counterfactual_routing.html'), label: 'Counterfactual Routing' }
        ]},
        { label: 'Model Layer', items: [
          { href: viewHref('cost_and_loops.html'), label: 'Token & Cost Burn' },
          { href: viewHref('message_mix.html'), label: 'Message & Turn Mix' }
        ]},
        { label: 'Harness Layer', items: [
          { href: viewHref('subagent_lineage.html'), label: 'Subagent Lineage' },
          { href: viewHref('process_resources.html'), label: 'Process & Resources' }
        ]},
        { label: 'Coordination', items: [
          { href: viewHref('seat_dynamics.html'), label: 'Actor Activity & Yield' },
          { href: viewHref('handoff_flow.html'), label: 'Handoff & Rework' },
          { href: viewHref('executive_governance.html'), label: 'Governance Scorecard' }
        ]},
        { label: 'Environment', items: [
          { href: viewHref('security_posture.html'), label: 'Security Posture' }
        ]}
      ];

      document.body.classList.add('dendro-shell');

      var sidebar = document.createElement('aside');
      sidebar.className = 'dendro-sidebar';
      sidebar.setAttribute('aria-label', 'Dendro');

      var brand = document.createElement('a');
      brand.className = 'dendro-sidebar-brand';
      brand.href = dash + 'index.html';
      var mark = document.createElement('span');
      mark.className = 'dendro-sidebar-mark';
      mark.innerHTML = '<svg class="brand-mark" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 128 128" width="28" height="28" role="img" aria-label="Dendro mark"><rect x="6" y="6" width="116" height="116" rx="26" fill="#0E1714" stroke="#243B33" stroke-width="3"/><g stroke="#29443A" stroke-width="3" stroke-linecap="round"><line x1="64" y1="14" x2="64" y2="24"/><line x1="64" y1="104" x2="64" y2="114"/><line x1="14" y1="64" x2="24" y2="64"/><line x1="104" y1="64" x2="114" y2="64"/></g><circle cx="64" cy="64" r="42" fill="none" stroke="#10B981" stroke-width="7" stroke-linecap="round" stroke-dasharray="212 52" transform="rotate(-35 64 64)"/><circle cx="64" cy="64" r="29" fill="none" stroke="#06B6D4" stroke-width="6.5" stroke-linecap="round" stroke-dasharray="138 44" transform="rotate(40 64 64)"/><circle cx="64" cy="64" r="17" fill="none" stroke="#F2A541" stroke-width="6" stroke-linecap="round" stroke-dasharray="78 29" transform="rotate(-110 64 64)"/><circle cx="64" cy="64" r="6.5" fill="#F4EFE6"/></svg>';
      var word = document.createElement('span');
      word.className = 'dendro-word';
      var name = document.createElement('strong');
      name.textContent = 'Dendro';
      var badge = document.createElement('span');
      badge.className = 'dendro-version';
      badge.textContent = 'Fleet Intelligence';
      word.appendChild(name);
      word.appendChild(badge);
      brand.appendChild(mark);
      brand.appendChild(word);

      var closeBtn = document.createElement('button');
      closeBtn.type = 'button';
      closeBtn.className = 'dendro-sidebar-close';
      closeBtn.textContent = 'Close';

      var brandRow = document.createElement('div');
      brandRow.className = 'dendro-sidebar-brandrow';
      brandRow.appendChild(brand);
      brandRow.appendChild(closeBtn);
      sidebar.appendChild(brandRow);

      var nav = document.createElement('nav');
      nav.className = 'dendro-sidenav';
      nav.setAttribute('aria-label', 'Intelligence views');
      groups.forEach(function (group) {
        var label = document.createElement('div');
        label.className = 'dendro-nav-label';
        label.textContent = group.label;
        nav.appendChild(label);
        group.items.forEach(function (item) {
          var link = document.createElement('a');
          link.className = 'dendro-nav-link';
          link.href = item.href;
          link.textContent = item.label;
          var target = item.href.split('/').pop();
          if (here && target === here) {
            link.classList.add('is-active');
            link.setAttribute('aria-current', 'page');
          }
          nav.appendChild(link);
        });
      });
      sidebar.appendChild(nav);

      var foot = document.createElement('div');
      foot.className = 'dendro-sidebar-foot';
      foot.textContent = 'Local-First Fleet Intelligence';
      sidebar.appendChild(foot);

      var backdrop = document.createElement('div');
      backdrop.className = 'dendro-sidebar-backdrop';
      backdrop.hidden = true;

      function setOpen(open) {
        sidebar.classList.toggle('is-open', open);
        backdrop.classList.toggle('is-open', open);
        backdrop.hidden = !open;
        var toggle = document.getElementById('dendro-sidebar-toggle');
        if (toggle) {
          toggle.setAttribute('aria-expanded', open ? 'true' : 'false');
          toggle.setAttribute('aria-label', open ? 'Close navigation' : 'Open navigation');
        }
      }

      closeBtn.addEventListener('click', function () { setOpen(false); });
      backdrop.addEventListener('click', function () { setOpen(false); });

      if (typeof document.querySelector === 'function') {
        var navLeft = document.querySelector('.telemetry-nav .nav-left');
        if (navLeft && !document.getElementById('dendro-sidebar-toggle')) {
          var toggle = document.createElement('button');
          toggle.type = 'button';
          toggle.id = 'dendro-sidebar-toggle';
          toggle.className = 'dendro-sidebar-toggle';
          toggle.setAttribute('aria-label', 'Open navigation');
          toggle.setAttribute('aria-expanded', 'false');
          var burger = document.createElement('span');
          burger.className = 'dendro-burger';
          burger.setAttribute('aria-hidden', 'true');
          toggle.appendChild(burger);
          toggle.addEventListener('click', function () {
            setOpen(!sidebar.classList.contains('is-open'));
          });
          navLeft.insertBefore(toggle, navLeft.firstChild);
        }

        var navRight = document.querySelector('.telemetry-nav .nav-right');
        if (navRight && !document.getElementById('nav-search-trigger')) {
          var searchBtn = document.createElement('button');
          searchBtn.type = 'button';
          searchBtn.id = 'nav-search-trigger';
          searchBtn.className = 'nav-search-trigger';
          searchBtn.title = 'Open Spotlight';
          var searchLabel = document.createElement('span');
          searchLabel.textContent = 'Spotlight';
          var kbd = document.createElement('kbd');
          kbd.textContent = '⌘K';
          searchBtn.appendChild(searchLabel);
          searchBtn.appendChild(kbd);
          searchBtn.addEventListener('click', function () {
            if (global.DendroUI && typeof global.DendroUI.openSpotlight === 'function') global.DendroUI.openSpotlight();
            else focusShellSearch();
          });
          navRight.insertBefore(searchBtn, navRight.firstChild);
        }

        if (navRight && !navRight.querySelector('a[href*="viewer.html"]')) {
          var explorer = document.createElement('a');
          explorer.className = 'nav-action-btn';
          explorer.href = root + 'viewer.html';
          explorer.textContent = 'Fleet Explorer';
          navRight.appendChild(explorer);
        }
      }

      document.body.insertBefore(backdrop, document.body.firstChild);
      document.body.insertBefore(sidebar, document.body.firstChild);

      if (document.body && typeof document.body.setAttribute === 'function') {
        document.body.setAttribute('data-dendro-keys', '1');
      }
    } catch (err) {
      if (typeof console !== 'undefined' && console.warn) console.warn('Dendro shell skipped', err);
    }
  }

  function focusShellSearch() {
    if (typeof document === 'undefined' || typeof document.getElementById !== 'function') return false;
    var search = document.getElementById('view-search')
      || document.getElementById('seat-search')
      || document.getElementById('table-search');
    if (!search && typeof document.querySelector === 'function') {
      search = document.querySelector('main input[type="text"]');
    }
    if (!search || typeof search.focus !== 'function') return false;
    search.focus();
    return true;
  }

  initSprings();
  bindGlobalKeys();
  bindConceptClicks();
  mountProductShell();
  bindWindowChrome();
  bootDashboardChrome();

  global.attachLiveSync = attachLiveSync;
  global.DendroLiveSync = attachLiveSync;
  attachLiveSync.normalizeViewPayload = normalizeViewPayload;
  attachLiveSync.openConceptSheet = openConceptSheet;
  attachLiveSync.openSpotlight = openSpotlight;
})(typeof window !== 'undefined' ? window : this);
