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
    let isLive = false;

    const badge = document.getElementById(config.badgeId);
    const badgeText = document.getElementById(config.badgeTextId);

    var SNAPSHOT_BADGE = 'ILLUSTRATIVE SNAPSHOT';
    var LIVE_BADGE = 'LIVE DUCKDB';
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
      return String(value);
    }

    function hideLivePanel() {
      if (typeof document === 'undefined' || typeof document.getElementById !== 'function') return;
      var panel = document.getElementById('live-duckdb-panel');
      if (panel) panel.hidden = true;
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
      title.textContent = 'Live DuckDB window';
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
      if (windowEl) windowEl.textContent = 'Live DuckDB window ' + from + ' → ' + to;

      var rows = Array.isArray(data.rows) ? data.rows : [];
      var total = typeof data.row_count === 'number' ? data.row_count : rows.length;
      var totalEl = document.getElementById('live-duckdb-totals');
      if (totalEl) totalEl.textContent = 'Total rows: ' + total.toLocaleString();

      var factsEl = document.getElementById('live-duckdb-facts');
      if (factsEl) {
        factsEl.replaceChildren();
        FACT_KEYS.forEach(function (key) {
          var li = document.createElement('li');
          var count = factCount(data.table_counts, key);
          var labels = {
            usage_facts: 'usage observations',
            message_facts: 'message facts',
            activity_facts: 'activity intervals',
            queue_facts: 'queue items',
            security_facts: 'security posture records',
            session_edges: 'session lineage edges',
            tool_facts: 'tool facts',
            skill_facts: 'skill facts'
          };
          li.textContent = key + ': ' + (count == null ? '—' : count.toLocaleString()) + (labels[key] ? ' ' + labels[key] : '');
          factsEl.append(li);
        });
      }

      var columns = [];
      var seen = {};
      rows.forEach(function (row) {
        if (!row || typeof row !== 'object' || Array.isArray(row)) return;
        Object.keys(row).forEach(function (key) {
          if (!seen[key]) {
            seen[key] = true;
            columns.push(key);
          }
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

    function updateBadge(mode) {
      if (!badge) return;
      badge.classList.remove('live', 'snapshot');
      if (mode === 'live') {
        badge.classList.add('live');
        if (badgeText) badgeText.textContent = LIVE_BADGE;
      } else {
        badge.classList.add('snapshot');
        if (badgeText) badgeText.textContent = SNAPSHOT_BADGE;
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
      if (source === 'sse' || source === 'poll') {
        try { renderLiveDuckDbPanel(data, source); }
        catch (err) { console.error('Dendro: live DuckDB panel failed', err); }
      } else hideLivePanel();
      if (typeof config.onData === 'function') {
        try {
          config.onData(data, source);
        } catch (e) {
          console.error('Dendro: Error in onData callback:', e);
        }
      }
    }

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
        }
      };
    }

    // If HTTP, attempt SSE first, then fall back to polling
    function trySSE() {
      try {
        const url = `${config.endpoint}?view=${encodeURIComponent(config.viewId)}`;
        sse = new EventSource(url);

        sse.onopen = function() {
          // The socket is up. The page is still showing the static snapshot
          // until a live payload hydrates, so the badge stays ILLUSTRATIVE SNAPSHOT.
        };

        sse.onmessage = function(event) {
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
          if (sse) {
            sse.close();
            sse = null;
          }
          isLive = false;
          // Fall back to polling or snapshot
          startPolling();
        };
      } catch (err) {
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
      try {
        const url = `${config.pollEndpoint}?view=${encodeURIComponent(config.viewId)}`;
        const res = await fetch(url, { cache: 'no-store' });
        if (!res.ok) throw new Error(`HTTP ${res.status}`);
        const data = await res.json();
        if (data !== null && data !== undefined) {
          isLive = true;
          updateBadge('live');
          triggerData(data, 'poll');
        }
      } catch (err) {
        // Fall back to embedded data
        isLive = false;
        updateBadge('snapshot');
        const data = loadEmbeddedData();
        if (data) triggerData(data, 'embedded-snapshot');
      }
    }

    // Start connection
    trySSE();

    // Trigger initial embedded data if available right away
    const initialData = loadEmbeddedData();
    if (initialData) triggerData(initialData, 'initial-cache');

    return {
      get isLive() { return isLive; },
      disconnect: function() {
        if (sse) { sse.close(); sse = null; }
        if (pollTimer) { clearInterval(pollTimer); pollTimer = null; }
      },
      refresh: function() {
        if (isHttp) pollOnce();
        else {
          const d = loadEmbeddedData();
          if (d) triggerData(d, 'manual-refresh');
        }
      }
    };
  }

  global.attachLiveSync = attachLiveSync;
})(typeof window !== 'undefined' ? window : this);
