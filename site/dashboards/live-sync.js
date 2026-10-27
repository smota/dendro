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
      if (!panel) return;
      if (typeof panel.getAttribute === 'function' && panel.getAttribute('data-keep-visible') === 'true') return;
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
      badge.textContent = 'v0.8 Intelligence';
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
      foot.textContent = 'DuckDB 2.0 LTS · Local-First';
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
          searchBtn.title = 'Focus search';
          var searchLabel = document.createElement('span');
          searchLabel.textContent = 'Search';
          var kbd = document.createElement('kbd');
          kbd.textContent = '/';
          searchBtn.appendChild(searchLabel);
          searchBtn.appendChild(kbd);
          searchBtn.addEventListener('click', focusShellSearch);
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

      if (typeof document.addEventListener === 'function' && !document.body.getAttribute('data-dendro-keys')) {
        document.body.setAttribute('data-dendro-keys', '1');
        document.addEventListener('keydown', function (event) {
          if (!event || event.key !== '/' || event.metaKey || event.ctrlKey || event.altKey) return;
          var target = event.target;
          if (!target || !target.tagName) return;
          var tag = target.tagName;
          if (tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT' || target.isContentEditable) return;
          if (focusShellSearch()) event.preventDefault();
        });
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

  mountProductShell();

  global.attachLiveSync = attachLiveSync;
})(typeof window !== 'undefined' ? window : this);
