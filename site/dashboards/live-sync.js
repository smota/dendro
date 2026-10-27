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

    function updateBadge(mode, label) {
      if (!badge) return;
      badge.classList.remove('live', 'snapshot');
      if (mode === 'live') {
        badge.classList.add('live');
        if (badgeText) badgeText.textContent = label || 'LIVE SSE · STREAMING ACTIVE';
      } else {
        badge.classList.add('snapshot');
        if (badgeText) badgeText.textContent = label || 'ILLUSTRATIVE SNAPSHOT · LIVE SYNC READY';
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
      updateBadge('snapshot', 'ILLUSTRATIVE SNAPSHOT · LIVE SYNC READY');
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
          isLive = true;
          updateBadge('live', 'LIVE SSE · STREAMING ACTIVE');
        };

        sse.onmessage = function(event) {
          try {
            const data = JSON.parse(event.data);
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
      updateBadge('snapshot', 'POLLING DUCKDB · LOCAL SERVER');
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
        updateBadge('live', 'LIVE HTTP · LOCAL DUCKDB ACTIVE');
        triggerData(data, 'poll');
      } catch (err) {
        // Fall back to embedded data
        updateBadge('snapshot', 'ILLUSTRATIVE SNAPSHOT · LIVE SYNC READY');
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
